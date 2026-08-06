use prose::{
    ApprovalMethod, ApprovalVia, AttestationDecision, AttestationInput, AttestationItemInput,
    AudienceTier, AuthorKind, ContentRef, ContextSpec, DraftCreate, Error, LengthConstraint,
    LengthUnit, PackDocument, Store, SurfaceMapping, VerbConstraints, VerbFamily, VerbSpec,
};
use std::{
    fs,
    process::{Command, Stdio},
};
use tempfile::TempDir;

fn pack(id: &str) -> PackDocument {
    PackDocument {
        id: id.into(),
        description: "Personal, user-authored material".into(),
        contexts: vec![ContextSpec {
            name: "code-review".into(),
            description: "Review context".into(),
            content: "Only comment on actionable defects.".into(),
        }],
        verbs: vec![VerbSpec {
            name: "tighten".into(),
            description: "User-defined editing operation".into(),
            family: VerbFamily::TransformSelection,
            instructions: "Remove repetition without changing meaning.".into(),
            context_bindings: vec!["code-review".into()],
            constraints: VerbConstraints {
                length: Some(LengthConstraint {
                    unit: LengthUnit::Words,
                    min: None,
                    max: Some(100),
                }),
                no_new_claims: true,
                preserve_meaning: true,
            },
        }],
        audience_tiers: vec![AudienceTier {
            name: "team".into(),
            description: "Colleagues".into(),
            requirements: vec!["human reviewed".into()],
        }],
        surface_mappings: vec![SurfaceMapping {
            surface: "github".into(),
            context: "code-review".into(),
            default_audience_tier: "team".into(),
        }],
    }
}

fn store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("data/prose.db")).unwrap();
    (dir, store)
}

#[test]
fn initializes_private_wal_store_and_reopens() {
    let (dir, store) = store();
    assert_eq!(store.journal_mode().unwrap(), "wal");
    assert!(store.foreign_keys_enabled().unwrap());
    assert_eq!(store.schema_version().unwrap(), 1);
    let path = store.path().to_owned();
    drop(store);
    assert!(Store::open(&path).is_ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.as_os_str().to_os_string();
            sidecar.push(suffix);
            let sidecar = std::path::PathBuf::from(sidecar);
            if sidecar.exists() {
                assert_eq!(
                    fs::metadata(sidecar).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }
    drop(dir);
}

#[test]
fn store_does_not_rewrite_existing_parent_and_rejects_unknown_database() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = dir.path().join("store.db");
    let store = Store::open(&path).unwrap();
    drop(store);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("DROP TRIGGER pack_revisions_no_update", [])
        .unwrap();
    drop(connection);
    assert!(matches!(Store::open(&path), Err(Error::Conflict(_))));

    let unknown = dir.path().join("unknown.db");
    let connection = rusqlite::Connection::open(&unknown).unwrap();
    connection
        .execute("CREATE TABLE unrelated(value)", [])
        .unwrap();
    drop(connection);
    assert!(matches!(Store::open(unknown), Err(Error::Conflict(_))));
}

#[test]
fn strict_input_and_numeric_boundaries_are_validation_errors() {
    let trailing = format!("{} true", serde_json::to_string(&pack("mine")).unwrap());
    assert!(serde_json::from_str::<PackDocument>(&trailing).is_err());
    let nested_unknown = r#"{"id":"mine","description":"x","contexts":[{"name":"c","description":"x","content":"x","extra":true}],"verbs":[],"audience_tiers":[],"surface_mappings":[]}"#;
    assert!(serde_json::from_str::<PackDocument>(nested_unknown).is_err());

    let (_dir, mut store) = store();
    store.create_pack(&pack("mine")).unwrap();
    assert!(matches!(
        store.get_pack("mine", Some(0)),
        Err(Error::Validation(_))
    ));
    assert!(matches!(
        store.update_pack(&pack("mine"), i64::MAX as u64 + 1),
        Err(Error::Validation(_))
    ));
    assert!(matches!(
        store.create_draft(&DraftCreate {
            id: Some("blank".into()),
            content: " \n\t".into(),
            context: None,
            verb: None,
            author_kind: AuthorKind::Human,
            provenance: serde_json::json!({}),
        }),
        Err(Error::Validation(_))
    ));
}

#[test]
fn pack_revisions_are_typed_ordered_append_only_and_optimistic() {
    let (_dir, mut store) = store();
    let mut b = pack("b-pack");
    store.create_pack(&b).unwrap();
    store.create_pack(&pack("a-pack")).unwrap();
    assert_eq!(
        store
            .list_packs()
            .unwrap()
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["a-pack", "b-pack"]
    );

    b.description = "revision two".into();
    b.contexts.push(ContextSpec {
        name: "second".into(),
        description: "Second".into(),
        content: "Material".into(),
    });
    assert_eq!(store.update_pack(&b, 1).unwrap().revision, 2);
    assert!(matches!(
        store.update_pack(&b, 1),
        Err(Error::Stale { actual: 2, .. })
    ));
    assert_eq!(
        store
            .get_pack("b-pack", Some(1))
            .unwrap()
            .document
            .contexts
            .len(),
        1
    );
    assert_eq!(
        store
            .get_pack("b-pack", None)
            .unwrap()
            .document
            .contexts
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["code-review", "second"]
    );

    let inspection = rusqlite::Connection::open(store.path()).unwrap();
    let root_error = inspection
        .execute("DELETE FROM packs WHERE id='a-pack'", [])
        .unwrap_err();
    assert!(root_error.to_string().contains("immutable"));
    let sql_error = inspection
        .execute("UPDATE context_revisions SET content='changed'", [])
        .unwrap_err();
    assert!(sql_error.to_string().contains("append-only"));
    assert!(store.delete_pack("b-pack", 1).is_err());
    assert!(store.delete_pack("b-pack", 2).unwrap().deleted);
    let tombstone = store.get_pack("b-pack", None).unwrap();
    assert!(tombstone.deleted);
    assert_eq!(tombstone.document.contexts.len(), 2);
    assert_eq!(store.list_packs().unwrap().len(), 1);
}

#[test]
fn validation_and_explicit_serving_are_deterministic() {
    let (_dir, mut store) = store();
    let mut invalid = pack("mine");
    invalid.surface_mappings[0].default_audience_tier = "missing".into();
    assert!(matches!(
        store.create_pack(&invalid),
        Err(Error::Validation(_))
    ));
    store.create_pack(&pack("mine")).unwrap();
    assert!(matches!(
        store.serve_context("code-review", None),
        Err(Error::Conflict(_))
    ));
    store.use_pack("mine").unwrap();
    assert_eq!(store.active_pack().unwrap().as_deref(), Some("mine"));
    let (id, rev, context) = store.serve_context("code-review", None).unwrap();
    assert_eq!(
        (id.as_str(), rev, context.name.as_str()),
        ("mine", 1, "code-review")
    );
    assert_eq!(
        store
            .serve_verb("tighten", Some("mine"))
            .unwrap()
            .2
            .instructions,
        "Remove repetition without changing meaning."
    );
    let surface = store.serve_surface("github", Some("mine")).unwrap();
    assert_eq!(surface.pack_revision, 1);
    assert_eq!(surface.context.name, "code-review");
    assert_eq!(surface.default_audience_tier.name, "team");
    store.create_pack(&pack("other")).unwrap();
    assert_eq!(store.serve_context("code-review", None).unwrap().0, "mine");
    assert_eq!(
        store.serve_context("code-review", Some("other")).unwrap().0,
        "other"
    );
    store.use_pack("other").unwrap();
    assert_eq!(store.active_pack().unwrap().as_deref(), Some("other"));
    let inspection = rusqlite::Connection::open(store.path()).unwrap();
    assert_eq!(
        inspection
            .query_row("SELECT count(*) FROM setting_events", [], |row| row
                .get::<_, u64>(0))
            .unwrap(),
        2
    );
    assert!(
        inspection
            .execute("DELETE FROM setting_events", [])
            .is_err()
    );
}

#[test]
fn separate_starter_pack_is_explicit_data_with_zero_voice_modules() {
    let document: PackDocument =
        serde_json::from_str(include_str!("../packs/starter.json")).unwrap();
    document.validate().unwrap();
    assert_eq!(document.id, "starter");
    assert!(document.contexts.is_empty());
    assert_eq!(
        document
            .verbs
            .iter()
            .map(|verb| verb.name.as_str())
            .collect::<Vec<_>>(),
        ["draft", "proofread", "shape", "tighten"]
    );
    assert!(
        document
            .verbs
            .iter()
            .all(|verb| verb.context_bindings.is_empty())
    );
}

#[test]
fn drafts_are_linear_and_historical_versions_remain_immutable() {
    let (_dir, mut store) = store();
    store.create_pack(&pack("mine")).unwrap();
    let input = DraftCreate {
        id: Some("draft-one".into()),
        content: "first".into(),
        context: Some(ContentRef {
            pack_id: "mine".into(),
            name: "code-review".into(),
        }),
        verb: Some(ContentRef {
            pack_id: "mine".into(),
            name: "tighten".into(),
        }),
        author_kind: AuthorKind::Agent,
        provenance: serde_json::json!({"session":"test"}),
    };
    assert_eq!(store.create_draft(&input).unwrap().version, 1);
    assert_eq!(
        store
            .revise_draft("draft-one", 1, "second")
            .unwrap()
            .parent_version,
        Some(1)
    );
    assert!(matches!(
        store.revise_draft("draft-one", 1, "branch"),
        Err(Error::Stale { actual: 2, .. })
    ));
    assert_eq!(
        store.get_draft("draft-one", Some(1)).unwrap().content,
        "first"
    );
    assert_eq!(
        store.get_draft("draft-one", None).unwrap().content,
        "second"
    );
    let inspection = rusqlite::Connection::open(store.path()).unwrap();
    assert!(
        inspection
            .execute("DELETE FROM draft_versions WHERE version=1", [])
            .unwrap_err()
            .to_string()
            .contains("append-only")
    );
}

#[test]
fn draft_provenance_freezes_exact_pack_revisions() {
    let (_dir, mut store) = store();
    let mut document = pack("mine");
    store.create_pack(&document).unwrap();
    let created = store
        .create_draft(&DraftCreate {
            id: Some("frozen".into()),
            content: "first".into(),
            context: Some(ContentRef {
                pack_id: "mine".into(),
                name: "code-review".into(),
            }),
            verb: Some(ContentRef {
                pack_id: "mine".into(),
                name: "tighten".into(),
            }),
            author_kind: AuthorKind::Agent,
            provenance: serde_json::json!({}),
        })
        .unwrap();
    assert_eq!(created.context.as_ref().unwrap().pack_revision, 1);
    assert_eq!(created.verb.as_ref().unwrap().pack_revision, 1);

    document.contexts[0].content = "Changed material".into();
    document.verbs[0].instructions = "Changed instructions".into();
    store.update_pack(&document, 1).unwrap();
    let frozen = store.get_draft("frozen", None).unwrap();
    assert_eq!(frozen.context.unwrap().pack_revision, 1);
    assert_eq!(frozen.verb.unwrap().pack_revision, 1);
    assert_eq!(
        store.get_pack("mine", Some(1)).unwrap().document.verbs[0].instructions,
        "Remove repetition without changing meaning."
    );
}

#[test]
fn semantic_validation_and_full_pack_round_trip_are_strict() {
    let (_dir, mut store) = store();
    let mut document = pack("mine");
    document.verbs.push(VerbSpec {
        name: "draft".into(),
        description: "Draft from supplied intent".into(),
        family: VerbFamily::DraftFromIntent,
        instructions: "Express only the supplied intent.".into(),
        context_bindings: vec!["code-review".into()],
        constraints: VerbConstraints {
            length: None,
            no_new_claims: true,
            preserve_meaning: true,
        },
    });
    document.verbs.sort_by(|a, b| a.name.cmp(&b.name));
    let stored = store.create_pack(&document).unwrap();
    assert_eq!(stored.document, document);
    assert_eq!(store.get_pack("mine", Some(1)).unwrap().document, document);

    let mut bad_binding = pack("bad-binding");
    bad_binding.verbs[0].context_bindings = vec!["missing".into()];
    assert!(
        matches!(store.create_pack(&bad_binding), Err(Error::Validation(message)) if message.contains("missing context"))
    );

    let mut bad_length = pack("bad-length");
    bad_length.verbs[0].constraints.length = Some(LengthConstraint {
        unit: LengthUnit::Characters,
        min: Some(20),
        max: Some(10),
    });
    assert!(
        matches!(store.create_pack(&bad_length), Err(Error::Validation(message)) if message.contains("min must not exceed max"))
    );

    let unknown = r#"{"name":"x","description":"x","family":"draft-from-intent","instructions":"x","context_bindings":[],"constraints":{"length":{"unit":"words","min":1,"max":2,"extra":true},"no_new_claims":false,"preserve_meaning":false}}"#;
    assert!(serde_json::from_str::<VerbSpec>(unknown).is_err());
    let unknown_item = r#"{"human_approved":true,"approved_by":"Chris","via":{"harness":"h","session":"s"},"method":"explicit","audience_tier":{"pack_id":"p","name":"t"},"items":[{"draft_id":"d","version":1,"exceptions":[],"extra":true}]}"#;
    assert!(serde_json::from_str::<AttestationInput>(unknown_item).is_err());
}

#[test]
fn missing_draft_errors_name_drafts_and_bulk_failure_is_atomic() {
    let (_dir, mut store) = store();
    store.create_pack(&pack("mine")).unwrap();
    let error = store.get_draft("absent", None).unwrap_err();
    assert_eq!(error.code(), "not_found");
    assert!(matches!(error, Error::NotFound { kind: "draft", .. }));

    store
        .create_draft(&DraftCreate {
            id: Some("present".into()),
            content: "text".into(),
            context: None,
            verb: None,
            author_kind: AuthorKind::Human,
            provenance: serde_json::json!({}),
        })
        .unwrap();
    assert!(matches!(
        store.get_draft("present", Some(2)),
        Err(Error::NotFound {
            kind: "draft version",
            ..
        })
    ));
    let input = AttestationInput {
        human_approved: true,
        approved_by: "Chris".into(),
        via: ApprovalVia {
            harness: "opencode".into(),
            session: "atomic".into(),
        },
        method: ApprovalMethod::Explicit,
        audience_tier: ContentRef {
            pack_id: "mine".into(),
            name: "team".into(),
        },
        items: vec![
            AttestationItemInput {
                draft_id: "present".into(),
                version: 1,
                decision: AttestationDecision::Approved,
                exceptions: vec![],
            },
            AttestationItemInput {
                draft_id: "absent".into(),
                version: 1,
                decision: AttestationDecision::Approved,
                exceptions: vec![],
            },
        ],
    };
    let error = store.attest(&input).unwrap_err();
    assert!(error.to_string().contains("draft"));
    assert!(store.list_attestations(None, None).unwrap().is_empty());
}

#[test]
fn independent_connections_resolve_competing_writes_optimistically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("concurrent.db");
    let mut first = Store::open(&path).unwrap();
    first.create_pack(&pack("mine")).unwrap();
    first
        .create_draft(&DraftCreate {
            id: Some("draft".into()),
            content: "one".into(),
            context: None,
            verb: None,
            author_kind: AuthorKind::Human,
            provenance: serde_json::json!({}),
        })
        .unwrap();
    let mut second = Store::open(&path).unwrap();
    let mut changed = pack("mine");
    changed.description = "winner".into();
    first.update_pack(&changed, 1).unwrap();
    assert!(matches!(
        second.update_pack(&changed, 1),
        Err(Error::Stale { actual: 2, .. })
    ));
    first.revise_draft("draft", 1, "two").unwrap();
    assert!(matches!(
        second.revise_draft("draft", 1, "branch"),
        Err(Error::Stale { actual: 2, .. })
    ));
    assert_eq!(second.get_draft("draft", None).unwrap().content, "two");
}

#[cfg(unix)]
#[test]
fn privacy_rejects_symbolic_and_hard_linked_store_paths() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.db");
    fs::write(&target, []).unwrap();
    let symbolic = dir.path().join("symbolic.db");
    symlink(&target, &symbolic).unwrap();
    assert!(matches!(
        Store::open(symbolic),
        Err(Error::Validation(_) | Error::Io(_))
    ));

    let hard = dir.path().join("hard.db");
    fs::hard_link(&target, &hard).unwrap();
    assert!(matches!(Store::open(hard), Err(Error::Io(_))));
}

#[test]
fn bulk_attestations_require_human_confirmation_and_exact_versions() {
    let (_dir, mut store) = store();
    store.create_pack(&pack("mine")).unwrap();
    for id in ["one", "two"] {
        store
            .create_draft(&DraftCreate {
                id: Some(id.into()),
                content: id.into(),
                context: None,
                verb: None,
                author_kind: AuthorKind::Human,
                provenance: serde_json::json!({}),
            })
            .unwrap();
    }
    let mut input = AttestationInput {
        human_approved: false,
        approved_by: "Chris".into(),
        via: ApprovalVia {
            harness: "opencode".into(),
            session: "session-42".into(),
        },
        method: ApprovalMethod::VerbalLgtm,
        audience_tier: ContentRef {
            pack_id: "mine".into(),
            name: "team".into(),
        },
        items: vec![
            AttestationItemInput {
                draft_id: "one".into(),
                version: 1,
                decision: AttestationDecision::Approved,
                exceptions: vec!["Use the shorter title".into()],
            },
            AttestationItemInput {
                draft_id: "two".into(),
                version: 1,
                decision: AttestationDecision::Approved,
                exceptions: vec![],
            },
        ],
    };
    assert!(matches!(store.attest(&input), Err(Error::Validation(_))));
    input.human_approved = true;
    input.items[0].decision = AttestationDecision::Excepted;
    let records = store.attest(&input).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].batch_id, records[1].batch_id);
    assert!(records[0].human_approved);
    assert_eq!(records[0].approved_by, "Chris");
    assert_eq!(records[0].via, input.via);
    assert_eq!(records[0].method, input.method);
    assert_eq!(records[0].decision, AttestationDecision::Excepted);
    assert_eq!(records[0].exceptions, ["Use the shorter title"]);
    assert_eq!(records[1].decision, AttestationDecision::Approved);
    assert_eq!(records[0].audience_tier.pack_revision, 1);
    assert_eq!(
        store.list_attestations(Some("one"), None).unwrap()[0]
            .draft
            .version,
        1
    );
    let inspection = rusqlite::Connection::open(store.path()).unwrap();
    inspection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    assert!(
        inspection
            .execute("DELETE FROM attestations", [])
            .unwrap_err()
            .to_string()
            .contains("immutable")
    );
    assert!(
        inspection
            .execute(
                "INSERT INTO attestations(id,batch_id,draft_id,draft_version,exceptions_json) VALUES('late',?1,'one',1,'[]')",
                [&records[0].batch_id],
            )
            .unwrap_err()
            .to_string()
            .contains("closed")
    );
    input.items[0].version = 99;
    assert!(matches!(store.attest(&input), Err(Error::NotFound { .. })));
}

fn run(args: &[&str], stdin: Option<&str>) -> std::process::Output {
    run_env(args, stdin, &[])
}

fn run_env(
    args: &[&str],
    stdin: Option<&str>,
    env: &[(&str, &std::path::Path)],
) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_prose"));
    child
        .args(args)
        .env_remove("PROSE_STORE")
        .env_remove("XDG_DATA_HOME")
        .env_remove("HOME");
    for (name, value) in env {
        child.env(name, value);
    }
    if stdin.is_some() {
        child.stdin(Stdio::piped());
    }
    let mut child = child
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(value) = stdin {
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(value.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

#[test]
fn cli_store_precedence_is_argument_then_environment_then_xdg() {
    let dir = tempfile::tempdir().unwrap();
    let xdg = dir.path().join("xdg");
    let env_store = dir.path().join("env.db");
    let cli_store = dir.path().join("cli.db");

    assert!(
        run_env(&["pack", "list"], None, &[("XDG_DATA_HOME", &xdg)])
            .status
            .success()
    );
    assert!(xdg.join("prose/prose.db").exists());

    assert!(
        run_env(
            &["pack", "list"],
            None,
            &[("XDG_DATA_HOME", &xdg), ("PROSE_STORE", &env_store)]
        )
        .status
        .success()
    );
    assert!(env_store.exists());

    assert!(
        run_env(
            &["--store", cli_store.to_str().unwrap(), "pack", "list"],
            None,
            &[("XDG_DATA_HOME", &xdg), ("PROSE_STORE", &env_store)]
        )
        .status
        .success()
    );
    assert!(cli_store.exists());
}

#[test]
fn cli_has_single_line_json_protocol_file_stdin_override_and_strict_schema() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("override.db");
    let input = serde_json::to_string(&pack("mine")).unwrap();
    let output = run(
        &["--json", "--store", db.to_str().unwrap(), "pack", "create"],
        Some(&input),
    );
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout).unwrap()["action"],
        "created"
    );

    let content = dir.path().join("content.txt");
    fs::write(&content, "second version").unwrap();
    let draft = r#"{"id":"cli-draft","content":"first","context":null,"verb":null}"#;
    assert!(
        run(
            &["--json", "--store", db.to_str().unwrap(), "draft", "create"],
            Some(draft)
        )
        .status
        .success()
    );
    assert!(
        run(
            &[
                "--store",
                db.to_str().unwrap(),
                "--json",
                "draft",
                "revise",
                "cli-draft",
                "--expected-version",
                "1",
                "--content",
                content.to_str().unwrap()
            ],
            None
        )
        .status
        .success()
    );

    let bad = "{\"id\":\"bad\",\"description\":\"x\",\"contexts\":[],\"verbs\":[],\"audience_tiers\":[],\"surface_mappings\":[],\"unknown\":true}".to_owned();
    let output = run(
        &["--json", "--store", db.to_str().unwrap(), "pack", "create"],
        Some(&bad),
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1);
    let error: serde_json::Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(error["error"]["code"], "invalid_input");

    let isolated = run(&["--json", "pack", "list"], None);
    assert!(!isolated.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&isolated.stderr).unwrap()["error"]["code"],
        "invalid_input"
    );
}

#[test]
fn cli_import_activate_enumerate_and_export_pack() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let starter = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/starter.json");
    let output = run(
        &[
            "--json",
            "--store",
            db.to_str().unwrap(),
            "pack",
            "import",
            "--file",
            starter.to_str().unwrap(),
            "--activate",
        ],
        None,
    );
    assert!(output.status.success(), "{output:?}");
    let imported: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(imported["action"], "imported");

    let output = run(&["--json", "--store", db.to_str().unwrap(), "verbs"], None);
    assert!(output.status.success(), "{output:?}");
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 4);

    let output = run(
        &[
            "--json",
            "--store",
            db.to_str().unwrap(),
            "pack",
            "export",
            "starter",
        ],
        None,
    );
    assert!(output.status.success(), "{output:?}");
    let exported: PackDocument = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(exported.id, "starter");
}
