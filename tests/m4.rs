use prose::{
    Application, ApprovalMethod, ApprovalVia, AttestationDecision, AttestationInput,
    AttestationItemInput, AudienceTier, AuthorKind, CaptureInput, CaptureObservation, ContextSpec,
    DraftCreate, ItemKind, PackDocument, PackRequest, Store, SurfaceMapping, VerbConstraints,
    VerbFamily, VerbSpec,
};
use serde_json::json;

fn pack() -> PackDocument {
    PackDocument {
        id: "mine".into(),
        description: "test".into(),
        contexts: vec![ContextSpec {
            name: "context".into(),
            description: "old metadata".into(),
            content: "old material".into(),
        }],
        verbs: vec![VerbSpec {
            name: "verb".into(),
            description: "old verb".into(),
            family: VerbFamily::DraftFromIntent,
            instructions: "old instructions".into(),
            context_bindings: vec!["context".into()],
            constraints: VerbConstraints {
                length: None,
                no_new_claims: true,
                preserve_meaning: true,
            },
        }],
        audience_tiers: vec![AudienceTier {
            name: "team".into(),
            description: "team".into(),
            requirements: vec![],
        }],
        surface_mappings: vec![SurfaceMapping {
            surface: "github".into(),
            context: "context".into(),
            default_audience_tier: "team".into(),
        }],
    }
}

fn capture(
    id: &str,
    observation: CaptureObservation,
    parent_id: Option<&str>,
    surface: &str,
    url: &str,
    content: &str,
) -> CaptureInput {
    CaptureInput {
        id: Some(id.into()),
        observation,
        parent_id: parent_id.map(str::to_owned),
        surface: surface.into(),
        url: url.into(),
        content: content.into(),
        draft: None,
        metadata: json!({"source":"test"}),
    }
}

#[test]
fn capture_confirmation_requires_matching_submit_attempt_and_preserves_exact_content() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().join("store.db")).unwrap();
    let attempt = capture(
        "attempt",
        CaptureObservation::SubmitAttempt,
        None,
        "github-pr",
        "https://GITHUB.com:443/o/r/pull/1?view=files#discussion",
        "attempted content",
    );
    let recorded_attempt = store.record_capture(&attempt).unwrap();
    assert_eq!(
        recorded_attempt.observation,
        CaptureObservation::SubmitAttempt
    );
    assert_eq!(recorded_attempt.parent_id, None);

    let confirmed = capture(
        "confirmed",
        CaptureObservation::SurfaceConfirmedPost,
        Some("attempt"),
        "github-pr",
        "https://github.com/o/r/pull/1?notification=1",
        " confirmed content\n",
    );
    let recorded = store.record_capture(&confirmed).unwrap();
    assert_eq!(recorded.parent_id.as_deref(), Some("attempt"));
    assert_eq!(recorded.content, " confirmed content\n");
    assert_eq!(store.record_capture(&confirmed).unwrap(), recorded);
    assert_eq!(store.list_captures(None).unwrap().len(), 2);
}

#[test]
fn capture_rejects_missing_wrong_or_mismatched_confirmation_parents() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().join("store.db")).unwrap();
    store
        .record_capture(&capture(
            "attempt",
            CaptureObservation::SubmitAttempt,
            None,
            "github-pr",
            "https://github.com/o/r/pull/1",
            "attempt",
        ))
        .unwrap();

    let attempt_with_parent = capture(
        "attempt-with-parent",
        CaptureObservation::SubmitAttempt,
        Some("attempt"),
        "github-pr",
        "https://github.com/o/r/pull/1",
        "attempt",
    );
    assert!(
        store
            .record_capture(&attempt_with_parent)
            .unwrap_err()
            .to_string()
            .contains("must not have a parent")
    );

    let no_parent = capture(
        "no-parent",
        CaptureObservation::SurfaceConfirmedPost,
        None,
        "github-pr",
        "https://github.com/o/r/pull/1",
        "post",
    );
    assert!(
        store
            .record_capture(&no_parent)
            .unwrap_err()
            .to_string()
            .contains("must reference")
    );
    let missing = capture(
        "missing",
        CaptureObservation::SurfaceConfirmedPost,
        Some("unknown"),
        "github-pr",
        "https://github.com/o/r/pull/1",
        "post",
    );
    assert!(matches!(
        store.record_capture(&missing),
        Err(prose::Error::NotFound { .. })
    ));
    let wrong_surface = capture(
        "wrong-surface",
        CaptureObservation::SurfaceConfirmedPost,
        Some("attempt"),
        "linear",
        "https://github.com/o/r/pull/1",
        "post",
    );
    assert!(
        store
            .record_capture(&wrong_surface)
            .unwrap_err()
            .to_string()
            .contains("surface")
    );
    let wrong_url = capture(
        "wrong-url",
        CaptureObservation::SurfaceConfirmedPost,
        Some("attempt"),
        "github-pr",
        "https://github.com/o/r/pull/2",
        "post",
    );
    assert!(
        store
            .record_capture(&wrong_url)
            .unwrap_err()
            .to_string()
            .contains("normalized URL")
    );

    let confirmed = capture(
        "confirmed",
        CaptureObservation::SurfaceConfirmedPost,
        Some("attempt"),
        "github-pr",
        "https://github.com/o/r/pull/1",
        "post",
    );
    store.record_capture(&confirmed).unwrap();
    let child_of_confirmation = capture(
        "child",
        CaptureObservation::SurfaceConfirmedPost,
        Some("confirmed"),
        "github-pr",
        "https://github.com/o/r/pull/1",
        "post",
    );
    assert!(
        store
            .record_capture(&child_of_confirmation)
            .unwrap_err()
            .to_string()
            .contains("submit-attempt")
    );
}

#[test]
fn capture_ids_conflict_when_observation_or_parent_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().join("store.db")).unwrap();
    let attempt = capture(
        "attempt",
        CaptureObservation::SubmitAttempt,
        None,
        "linear",
        "https://linear.app/acme/issue/ONE-1",
        "content",
    );
    store.record_capture(&attempt).unwrap();

    let mut changed_observation = attempt.clone();
    changed_observation.observation = CaptureObservation::SurfaceConfirmedPost;
    changed_observation.parent_id = Some("other".into());
    assert!(matches!(
        store.record_capture(&changed_observation),
        Err(prose::Error::Conflict(_))
    ));
    let mut changed_parent = attempt.clone();
    changed_parent.parent_id = Some("other".into());
    assert!(matches!(
        store.record_capture(&changed_parent),
        Err(prose::Error::Conflict(_))
    ));
    assert_eq!(store.list_captures(None).unwrap().len(), 1);
}

#[test]
fn sqlite_enforces_confirmed_post_parent_invariants() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    drop(Store::open(&path).unwrap());
    let connection = rusqlite::Connection::open(path).unwrap();
    let insert = |id: &str,
                  observation: &str,
                  parent: Option<&str>,
                  surface: &str,
                  normalized: &str| {
        connection.execute(
            "INSERT INTO capture_events(id,observation,parent_id,normalized_url_key,surface,url,content,metadata_json) VALUES(?1,?2,?3,?4,?5,'https://example.test/post','body','{}')",
            rusqlite::params![id, observation, parent, normalized, surface],
        )
    };
    insert(
        "attempt",
        "submit-attempt",
        None,
        "github",
        "https://example.test/post",
    )
    .unwrap();
    assert!(
        insert(
            "self",
            "surface-confirmed-post",
            Some("self"),
            "github",
            "https://example.test/post"
        )
        .is_err()
    );
    assert!(
        insert(
            "missing",
            "surface-confirmed-post",
            Some("absent"),
            "github",
            "https://example.test/post"
        )
        .is_err()
    );
    insert(
        "confirmed",
        "surface-confirmed-post",
        Some("attempt"),
        "github",
        "https://example.test/post",
    )
    .unwrap();
    assert!(
        insert(
            "child",
            "surface-confirmed-post",
            Some("confirmed"),
            "github",
            "https://example.test/post"
        )
        .is_err()
    );
    assert!(
        insert(
            "surface",
            "surface-confirmed-post",
            Some("attempt"),
            "linear",
            "https://example.test/post"
        )
        .is_err()
    );
    assert!(
        insert(
            "url",
            "surface-confirmed-post",
            Some("attempt"),
            "github",
            "https://example.test/other"
        )
        .is_err()
    );
    assert!(connection.execute("INSERT INTO capture_events(id,surface,url,content,metadata_json) VALUES('empty-key','github','https://example.test/post','body','{}')", []).is_err());
}

#[test]
fn item_put_is_an_unlabelled_external_import_and_is_served_with_origin() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut store = Store::open(&path).unwrap();
    store.create_pack(&pack()).unwrap();
    drop(store);
    let app = Application::new(&path);

    app.pack(PackRequest::ItemPut {
        pack_id: "mine".into(),
        kind: ItemKind::Context,
        id: "context".into(),
        document: json!({
            "name":"context", "description":"caller metadata", "content":"new material"
        }),
        expected_revision: 1,
    })
    .unwrap();

    let served = app
        .context(prose::ContextRequest {
            name: "context".into(),
            pack: Some("mine".into()),
        })
        .unwrap();
    assert_eq!(served["context"]["content"], "new material");
    assert_eq!(served["origin"]["kind"], "external-import");
    assert_eq!(served["origin"]["source_label"], serde_json::Value::Null);
    assert_eq!(served["origin"]["source_uri"], serde_json::Value::Null);
    assert_eq!(served["origin"]["channel"], "item-put");
    assert_eq!(
        served["origin"]["material_sha256"].as_str().unwrap().len(),
        64
    );
    assert!(
        served["origin"]["recorded_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );

    let inspection = rusqlite::Connection::open(path).unwrap();
    assert!(
        inspection
            .execute("UPDATE pack_item_origins SET kind='external-import'", [])
            .is_err()
    );
    assert!(
        inspection
            .execute("DELETE FROM context_revision_origins", [])
            .is_err()
    );
}

#[test]
fn approved_install_uses_exact_attested_draft_for_any_author_and_keeps_typed_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut store = Store::open(&path).unwrap();
    store.create_pack(&pack()).unwrap();
    store
        .create_draft(&DraftCreate {
            id: Some("agent-draft".into()),
            content: "approved exact material".into(),
            context: None,
            verb: None,
            author_kind: AuthorKind::Agent,
            provenance: json!({}),
        })
        .unwrap();
    store
        .create_draft(&DraftCreate {
            id: Some("wrong-draft".into()),
            content: "different approved material".into(),
            context: None,
            verb: None,
            author_kind: AuthorKind::Human,
            provenance: json!({}),
        })
        .unwrap();
    store
        .create_draft(&DraftCreate {
            id: Some("same-content".into()),
            content: "approved exact material".into(),
            context: None,
            verb: None,
            author_kind: AuthorKind::Human,
            provenance: json!({}),
        })
        .unwrap();
    let attestations = store
        .attest(&AttestationInput {
            human_approved: true,
            approved_by: "test-approver".into(),
            via: ApprovalVia {
                harness: "test".into(),
                session: "one".into(),
            },
            method: ApprovalMethod::Explicit,
            audience_tier: prose::ContentRef {
                pack_id: "mine".into(),
                name: "team".into(),
            },
            items: vec![
                AttestationItemInput {
                    draft_id: "agent-draft".into(),
                    version: 1,
                    decision: AttestationDecision::Approved,
                    exceptions: vec![],
                },
                AttestationItemInput {
                    draft_id: "same-content".into(),
                    version: 1,
                    decision: AttestationDecision::Excepted,
                    exceptions: vec!["not approved for installation".into()],
                },
                AttestationItemInput {
                    draft_id: "wrong-draft".into(),
                    version: 1,
                    decision: AttestationDecision::Approved,
                    exceptions: vec![],
                },
            ],
        })
        .unwrap();
    let attestation = attestations
        .iter()
        .find(|item| item.draft.draft_id == "agent-draft")
        .unwrap()
        .clone();
    let excepted_id = attestations
        .iter()
        .find(|item| item.draft.draft_id == "same-content")
        .unwrap()
        .id
        .clone();
    let wrong_id = attestations
        .iter()
        .find(|item| item.draft.draft_id == "wrong-draft")
        .unwrap()
        .id
        .clone();
    drop(store);

    let app = Application::new(&path);
    let caller_document = json!({
        "name":"verb", "description":"caller supplied metadata",
        "family":"draft-from-intent", "instructions":"must not win",
        "context_bindings":["context"],
        "constraints":{"length":null,"no_new_claims":false,"preserve_meaning":true}
    });
    assert!(
        app.pack(PackRequest::ItemInstallApprovedDraft {
            pack_id: "mine".into(),
            kind: ItemKind::Verb,
            id: "verb".into(),
            document: caller_document.clone(),
            expected_revision: 1,
            attestation_id: "missing-attestation".into(),
            source: prose::DraftTarget {
                draft_id: "agent-draft".into(),
                version: 1,
            },
        })
        .is_err()
    );
    assert!(
        app.pack(PackRequest::ItemInstallApprovedDraft {
            pack_id: "mine".into(),
            kind: ItemKind::Verb,
            id: "verb".into(),
            document: caller_document.clone(),
            expected_revision: 1,
            attestation_id: wrong_id,
            source: prose::DraftTarget {
                draft_id: "agent-draft".into(),
                version: 1,
            },
        })
        .is_err()
    );
    assert!(
        app.pack(PackRequest::ItemInstallApprovedDraft {
            pack_id: "mine".into(),
            kind: ItemKind::Verb,
            id: "verb".into(),
            document: caller_document.clone(),
            expected_revision: 1,
            attestation_id: excepted_id,
            source: prose::DraftTarget {
                draft_id: "same-content".into(),
                version: 1,
            },
        })
        .is_err()
    );
    app.pack(PackRequest::ItemInstallApprovedDraft {
        pack_id: "mine".into(),
        kind: ItemKind::Verb,
        id: "verb".into(),
        document: caller_document,
        expected_revision: 1,
        attestation_id: attestation.id.clone(),
        source: prose::DraftTarget {
            draft_id: "agent-draft".into(),
            version: 1,
        },
    })
    .unwrap();
    let served = app
        .verb(prose::VerbRequest::Get {
            name: "verb".into(),
            pack: Some("mine".into()),
        })
        .unwrap();
    assert_eq!(served["verb"]["instructions"], "approved exact material");
    assert_eq!(served["verb"]["description"], "caller supplied metadata");
    assert_eq!(served["verb"]["constraints"]["no_new_claims"], false);
    assert_eq!(served["origin"]["kind"], "approved-draft");
    assert_eq!(served["origin"]["attestation_id"], attestation.id);
    assert_eq!(served["origin"]["draft_id"], "agent-draft");
    assert_eq!(served["origin"]["draft_version"], 1);

    assert!(
        app.pack(PackRequest::ItemInstallApprovedDraft {
            pack_id: "mine".into(),
            kind: ItemKind::Context,
            id: "context".into(),
            document: json!({"name":"context","description":"context metadata","content":"must not win"}),
            expected_revision: 2,
            attestation_id: "missing-attestation".into(),
            source: prose::DraftTarget {
                draft_id: "agent-draft".into(),
                version: 1,
            },
        })
        .is_err()
    );
    app.pack(PackRequest::ItemInstallApprovedDraft {
        pack_id: "mine".into(),
        kind: ItemKind::Context,
        id: "context".into(),
        document: json!({"name":"context","description":"context metadata","content":"must not win"}),
        expected_revision: 2,
        attestation_id: attestation.id.clone(),
        source: prose::DraftTarget {
            draft_id: "agent-draft".into(),
            version: 1,
        },
    })
    .unwrap();
    let context = app
        .context(prose::ContextRequest {
            name: "context".into(),
            pack: Some("mine".into()),
        })
        .unwrap();
    assert_eq!(context["context"]["content"], "approved exact material");
    assert_eq!(context["context"]["description"], "context metadata");
    assert_eq!(context["origin"]["attestation_id"], attestation.id);

    let mut store = Store::open(&path).unwrap();
    let mut direct = store.get_pack("mine", None).unwrap().document;
    direct.contexts[0].content = "direct caller material must not win".into();
    let installed = store
        .update_pack_item_approved(
            &direct,
            3,
            ItemKind::Context,
            "context",
            &attestation.id,
            &prose::DraftTarget {
                draft_id: "agent-draft".into(),
                version: 1,
            },
        )
        .unwrap();
    assert_eq!(
        installed.document.contexts[0].content,
        "approved exact material"
    );
    let mut direct_verb = installed.document.clone();
    direct_verb.verbs[0].instructions = "direct caller instructions must not win".into();
    let installed = store
        .update_pack_item_approved(
            &direct_verb,
            4,
            ItemKind::Verb,
            "verb",
            &attestation.id,
            &prose::DraftTarget {
                draft_id: "agent-draft".into(),
                version: 1,
            },
        )
        .unwrap();
    assert_eq!(
        installed.document.verbs[0].instructions,
        "approved exact material"
    );
    for kind in [ItemKind::Tier, ItemKind::Surface] {
        assert!(
            store
                .update_pack_item_approved(
                    &installed.document,
                    5,
                    kind,
                    if kind == ItemKind::Tier {
                        "team"
                    } else {
                        "github"
                    },
                    &attestation.id,
                    &prose::DraftTarget {
                        draft_id: "agent-draft".into(),
                        version: 1,
                    },
                )
                .unwrap_err()
                .to_string()
                .contains("only context and verb")
        );
    }
    assert_eq!(store.get_pack("mine", None).unwrap().revision, 5);
}

#[test]
fn application_inspects_every_item_origin_and_surface_constituents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    Store::open(&path).unwrap().create_pack(&pack()).unwrap();
    let app = Application::new(path);
    for (kind, id) in [
        (ItemKind::Context, "context"),
        (ItemKind::Verb, "verb"),
        (ItemKind::Tier, "team"),
        (ItemKind::Surface, "github"),
    ] {
        let value = app
            .pack(PackRequest::ItemOrigin {
                pack_id: "mine".into(),
                kind,
                id: id.into(),
                revision: None,
            })
            .unwrap();
        assert_eq!(value["pack_revision"], 1);
        assert_eq!(value["id"], id);
        assert_eq!(value["origin"]["kind"], "external-import");
    }
    let exact = app
        .pack(PackRequest::ItemOrigin {
            pack_id: "mine".into(),
            kind: ItemKind::Surface,
            id: "github".into(),
            revision: Some(1),
        })
        .unwrap();
    assert_eq!(exact["kind"], "surface");
    let surface = app
        .surface(prose::SurfaceRequest {
            name: "github".into(),
            pack: Some("mine".into()),
        })
        .unwrap();
    assert_eq!(surface["surface_origin"]["kind"], "external-import");
    assert_eq!(surface["context_origin"]["kind"], "external-import");
    assert_eq!(
        surface["default_audience_tier_origin"]["kind"],
        "external-import"
    );
}

#[test]
fn post_v3_snapshots_have_complete_origins_and_preserve_only_exact_unchanged_items() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut store = Store::open(&path).unwrap();
    let mut document = pack();
    store.create_pack(&document).unwrap();

    for (kind, key) in [
        (ItemKind::Context, "context"),
        (ItemKind::Verb, "verb"),
        (ItemKind::Tier, "team"),
        (ItemKind::Surface, "github"),
    ] {
        assert!(matches!(
            store.item_origin("mine", 1, kind, key).unwrap(),
            Some(prose::PackItemOrigin::ExternalImport { channel, .. }) if channel == "store-create"
        ));
    }
    let inspection = rusqlite::Connection::open(&path).unwrap();
    let origin_id = |table: &str, revision: u64, key_column: &str, key: &str| -> i64 {
        inspection
            .query_row(
                &format!("SELECT origin_id FROM {table} WHERE pack_id='mine' AND pack_revision=?1 AND {key_column}=?2"),
                rusqlite::params![revision, key],
                |row| row.get(0),
            )
            .unwrap()
    };
    let context_v1 = origin_id("context_revision_origins", 1, "name", "context");
    let verb_v1 = origin_id("verb_revision_origins", 1, "name", "verb");

    document.contexts[0].content = "changed".into();
    document.contexts.push(ContextSpec {
        name: "new-context".into(),
        description: "new".into(),
        content: "new".into(),
    });
    store.update_pack(&document, 1).unwrap();
    let context_v2 = origin_id("context_revision_origins", 2, "name", "context");
    assert_ne!(context_v1, context_v2);
    assert_eq!(
        verb_v1,
        origin_id("verb_revision_origins", 2, "name", "verb")
    );
    assert!(matches!(
        store.item_origin("mine", 2, ItemKind::Context, "context").unwrap(),
        Some(prose::PackItemOrigin::ExternalImport { channel, .. }) if channel == "store-update"
    ));
    assert!(matches!(
        store.item_origin("mine", 2, ItemKind::Context, "new-context").unwrap(),
        Some(prose::PackItemOrigin::ExternalImport { channel, .. }) if channel == "store-update"
    ));

    store.delete_pack("mine", 2).unwrap();
    assert_eq!(
        context_v2,
        origin_id("context_revision_origins", 3, "name", "context")
    );
    assert_eq!(
        verb_v1,
        origin_id("verb_revision_origins", 3, "name", "verb")
    );
    assert!(
        inspection
            .execute("UPDATE verb_revision_origins SET origin_id=origin_id", [])
            .is_err()
    );
}

#[test]
fn pack_import_channels_changes_and_typed_digest_ignores_json_spelling_and_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let app = Application::new(&path);
    let mut first = pack();
    first.id = "first".into();
    let mut second = first.clone();
    second.id = "second".into();
    app.pack(PackRequest::Import {
        document: first,
        expected_revision: None,
        activate: false,
    })
    .unwrap();
    app.pack(PackRequest::Import {
        document: second,
        expected_revision: None,
        activate: false,
    })
    .unwrap();
    let store = Store::open(&path).unwrap();
    assert!(matches!(
        store.item_origin("first", 1, ItemKind::Verb, "verb").unwrap(),
        Some(prose::PackItemOrigin::ExternalImport { channel, .. }) if channel == "pack-import"
    ));
    drop(store);

    let omitted_default = json!({
        "name":"verb","description":"same","family":"draft-from-intent",
        "instructions":"same","constraints":{"length":null,"no_new_claims":true,"preserve_meaning":true}
    });
    let explicit_default = json!({
        "constraints":{"preserve_meaning":true,"no_new_claims":true,"length":null},
        "context_bindings":[],"instructions":"same","family":"draft-from-intent",
        "description":"same","name":"verb"
    });
    app.pack(PackRequest::ItemPut {
        pack_id: "first".into(),
        kind: ItemKind::Verb,
        id: "verb".into(),
        document: omitted_default,
        expected_revision: 1,
    })
    .unwrap();
    app.pack(PackRequest::ItemPut {
        pack_id: "second".into(),
        kind: ItemKind::Verb,
        id: "verb".into(),
        document: explicit_default,
        expected_revision: 1,
    })
    .unwrap();
    let store = Store::open(&path).unwrap();
    let digest = |pack_id: &str| match store
        .item_origin(pack_id, 2, ItemKind::Verb, "verb")
        .unwrap()
        .unwrap()
    {
        prose::PackItemOrigin::ExternalImport {
            material_sha256, ..
        } => material_sha256,
        _ => panic!("expected external import"),
    };
    assert_eq!(digest("first"), digest("second"));

    let mut imported = store.get_pack("first", None).unwrap().document;
    imported.description = "updated pack".into();
    imported.contexts[0].content = "changed by import".into();
    drop(store);
    app.pack(PackRequest::Import {
        document: imported,
        expected_revision: Some(2),
        activate: false,
    })
    .unwrap();
    let store = Store::open(path).unwrap();
    assert_eq!(
        store
            .item_origin("first", 2, ItemKind::Verb, "verb")
            .unwrap(),
        store
            .item_origin("first", 3, ItemKind::Verb, "verb")
            .unwrap()
    );
    assert!(matches!(
        store.item_origin("first", 3, ItemKind::Context, "context").unwrap(),
        Some(prose::PackItemOrigin::ExternalImport { channel, .. }) if channel == "pack-import"
    ));
}

#[test]
fn schema_v2_migration_is_additive_and_does_not_infer_legacy_origins() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut store = Store::open(&path).unwrap();
    store.create_pack(&pack()).unwrap();
    store
        .record_capture(&capture(
            "legacy",
            CaptureObservation::SubmitAttempt,
            None,
            "linear",
            "https://linear.app/acme/issue/ONE-1",
            "legacy attempt",
        ))
        .unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER approved_origin_requires_approval;
         DROP TRIGGER approved_origin_kind; DROP TRIGGER external_origin_kind;
         DROP TRIGGER context_origin_complete; DROP TRIGGER context_approved_material;
         DROP TRIGGER verb_origin_complete; DROP TRIGGER verb_approved_material;
         DROP TRIGGER tier_origin_complete; DROP TRIGGER tier_rejects_approved_origin;
         DROP TRIGGER surface_origin_complete; DROP TRIGGER surface_rejects_approved_origin;
         DROP TRIGGER origins_no_update; DROP TRIGGER origins_no_delete;
         DROP TRIGGER approved_origins_no_update; DROP TRIGGER approved_origins_no_delete;
         DROP TRIGGER external_origins_no_update; DROP TRIGGER external_origins_no_delete;
         DROP TRIGGER context_origins_no_update; DROP TRIGGER context_origins_no_delete;
         DROP TRIGGER verb_origins_no_update; DROP TRIGGER verb_origins_no_delete;
         DROP TRIGGER tier_origins_no_update; DROP TRIGGER tier_origins_no_delete;
         DROP TRIGGER surface_origins_no_update; DROP TRIGGER surface_origins_no_delete;
         DROP TABLE context_revision_origins; DROP TABLE verb_revision_origins;
         DROP TABLE audience_tier_revision_origins; DROP TABLE surface_mapping_revision_origins;
         DROP TABLE approved_draft_origins; DROP TABLE external_import_origins;
         DROP TABLE pack_item_origins; DROP INDEX attestations_exact_draft;
         DROP TRIGGER captures_no_update; DROP TRIGGER captures_no_delete;
         DROP TRIGGER captures_normalized_url_required;
         DROP TRIGGER capture_confirmation_parent_distinct;
         DROP TRIGGER capture_confirmation_parent_exists;
         DROP TRIGGER capture_confirmation_parent_attempt;
         DROP TRIGGER capture_confirmation_parent_target;
         ALTER TABLE capture_events RENAME TO capture_events_v3;
         CREATE TABLE capture_events (
           sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
           surface TEXT NOT NULL, url TEXT NOT NULL, content TEXT NOT NULL,
           draft_id TEXT, draft_version INTEGER,
           metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json) AND json_type(metadata_json)='object'),
           captured_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
           CHECK((draft_id IS NULL) = (draft_version IS NULL)),
           FOREIGN KEY(draft_id,draft_version) REFERENCES draft_versions(draft_id,version)
         ) STRICT;
          INSERT INTO capture_events(sequence,id,surface,url,content,draft_id,draft_version,metadata_json,captured_at)
            SELECT sequence,id,surface,'/legacy',content,draft_id,draft_version,metadata_json,captured_at
            FROM capture_events_v3;
         DROP TABLE capture_events_v3;
         CREATE TRIGGER captures_no_update BEFORE UPDATE ON capture_events BEGIN SELECT RAISE(ABORT,'immutable table'); END;
         CREATE TRIGGER captures_no_delete BEFORE DELETE ON capture_events BEGIN SELECT RAISE(ABORT,'immutable table'); END;
         DELETE FROM schema_migrations WHERE version=3; PRAGMA user_version=2;",
        )
        .unwrap();
    drop(connection);

    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), 3);
    assert_eq!(store.get_pack("mine", None).unwrap().document, pack());
    let legacy = store.get_capture("legacy").unwrap();
    assert_eq!(legacy.observation, CaptureObservation::SubmitAttempt);
    assert_eq!(legacy.parent_id, None);
    let normalized: String = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT normalized_url_key FROM capture_events WHERE id='legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy.url, "/legacy");
    assert_eq!(normalized, "legacy:/legacy");
    assert_eq!(
        store
            .item_origin("mine", 1, ItemKind::Context, "context")
            .unwrap(),
        None
    );
}
