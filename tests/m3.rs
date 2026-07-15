use prose::{
    Application, AssistPreparation, AuthorKind, CaptureInput, CaptureObservation, ContentRef,
    ContextSpec, DraftCreate, Error, PackDocument, Store, VerbConstraints, VerbFamily, VerbSpec,
    server::{ServeConfig, run, run_with_shutdown},
};
use serde_json::{Value, json};
use std::{
    net::{IpAddr, Ipv4Addr, TcpListener},
    path::Path,
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::oneshot,
};

fn port() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.local_addr().unwrap().port()
}

async fn request(port: u16, method: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
    let (status, _, body) = request_with_headers(port, method, path, body, &[]).await;
    (status, serde_json::from_str(&body).unwrap())
}

async fn request_with_headers(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&Value>,
    headers: &[(&str, &str)],
) -> (u16, String, String) {
    let body = body.map(Value::to_string).unwrap_or_default();
    let mut stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let extra = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect::<String>();
    let wire = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(wire.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, head.to_owned(), body.to_owned())
}

async fn wait(port: u16) {
    for _ in 0..100 {
        if tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("server did not start");
}

fn seeded(path: &Path) {
    let mut store = Store::open(path).unwrap();
    store
        .create_pack(&PackDocument {
            id: "test".into(),
            description: "test".into(),
            contexts: vec![ContextSpec {
                name: "code-review".into(),
                description: "review".into(),
                content: "Be direct.".into(),
            }],
            verbs: vec![VerbSpec {
                name: "tighten".into(),
                description: "tighten".into(),
                family: VerbFamily::TransformSelection,
                instructions: "Make it shorter.".into(),
                context_bindings: vec!["code-review".into()],
                constraints: VerbConstraints {
                    length: None,
                    no_new_claims: true,
                    preserve_meaning: true,
                },
            }],
            audience_tiers: vec![],
            surface_mappings: vec![],
        })
        .unwrap();
    store.use_pack("test").unwrap();
}

fn content_ref(name: &str) -> ContentRef {
    ContentRef {
        pack_id: "test".into(),
        name: name.into(),
    }
}

fn revisioned_pack(revision: u64) -> PackDocument {
    PackDocument {
        id: "race".into(),
        description: format!("revision {revision}"),
        contexts: vec![ContextSpec {
            name: "context".into(),
            description: "context".into(),
            content: format!("context material {revision}"),
        }],
        verbs: vec![VerbSpec {
            name: "verb".into(),
            description: "verb".into(),
            family: VerbFamily::TransformSelection,
            instructions: format!("verb material {revision}"),
            context_bindings: vec!["context".into()],
            constraints: VerbConstraints {
                length: None,
                no_new_claims: true,
                preserve_meaning: true,
            },
        }],
        audience_tiers: vec![],
        surface_mappings: vec![],
    }
}

#[test]
fn assist_preparation_freezes_material_and_rejects_incompatible_drafts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    seeded(&path);
    let app = Application::new(&path);
    let prepared = app
        .prepare_assist(&AssistPreparation {
            surface: "github-pr".into(),
            context: "code-review".into(),
            verb: "tighten".into(),
            pack: Some("test".into()),
            selection: "source".into(),
            surrounding_context: "surrounding".into(),
            draft: None,
        })
        .unwrap();
    assert_eq!(prepared.pack_id, "test");
    assert_eq!(prepared.pack_revision, 1);
    assert_eq!(prepared.context_text, "Be direct.");
    assert_eq!(prepared.verb_instructions, "Make it shorter.");
    assert_eq!(prepared.context.pack_revision, prepared.pack_revision);
    assert_eq!(prepared.verb.pack_revision, prepared.pack_revision);
    let source = Store::open(&path)
        .unwrap()
        .get_draft(&prepared.source.draft_id, Some(prepared.source.version))
        .unwrap();
    assert_eq!(source.context.as_ref(), Some(&prepared.context));
    assert_eq!(source.verb.as_ref(), Some(&prepared.verb));

    let mut store = Store::open(&path).unwrap();
    let mut changed = store.get_pack("test", None).unwrap().document;
    changed.contexts[0].content = "new context".into();
    changed.verbs[0].instructions = "new verb".into();
    store.update_pack(&changed, 1).unwrap();
    drop(store);
    let error = app
        .prepare_assist(&AssistPreparation {
            surface: "github-pr".into(),
            context: "code-review".into(),
            verb: "tighten".into(),
            pack: Some("test".into()),
            selection: "replacement".into(),
            surrounding_context: String::new(),
            draft: Some(prepared.source.clone()),
        })
        .unwrap_err();
    assert!(matches!(error, Error::Validation(message) if message.contains("refs do not match")));
    assert_eq!(
        Store::open(&path)
            .unwrap()
            .list_draft_versions(&prepared.source.draft_id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn concurrent_pack_revisions_never_mix_assist_material_or_refs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut store = Store::open(&path).unwrap();
    store.create_pack(&revisioned_pack(1)).unwrap();
    store.use_pack("race").unwrap();
    drop(store);

    let barrier = Arc::new(Barrier::new(3));
    let writer_path = path.clone();
    let writer_barrier = Arc::clone(&barrier);
    let writer = thread::spawn(move || {
        writer_barrier.wait();
        let mut store = Store::open(writer_path).unwrap();
        for revision in 2..=24 {
            store
                .update_pack(&revisioned_pack(revision), revision - 1)
                .unwrap();
            thread::yield_now();
        }
    });
    let reader_path = path.clone();
    let reader_barrier = Arc::clone(&barrier);
    let reader = thread::spawn(move || {
        reader_barrier.wait();
        let app = Application::new(&reader_path);
        for attempt in 0..24 {
            let prepared = app
                .prepare_assist(&AssistPreparation {
                    surface: "surface".into(),
                    context: "context".into(),
                    verb: "verb".into(),
                    pack: None,
                    selection: format!("source {attempt}"),
                    surrounding_context: String::new(),
                    draft: None,
                })
                .unwrap();
            let revision = prepared.pack_revision;
            assert_eq!(prepared.pack_id, "race");
            assert_eq!(
                prepared.context_text,
                format!("context material {revision}")
            );
            assert_eq!(
                prepared.verb_instructions,
                format!("verb material {revision}")
            );
            assert_eq!(prepared.context.pack_revision, revision);
            assert_eq!(prepared.verb.pack_revision, revision);
            let source = Store::open(&reader_path)
                .unwrap()
                .get_draft(&prepared.source.draft_id, Some(prepared.source.version))
                .unwrap();
            assert_eq!(source.context, Some(prepared.context));
            assert_eq!(source.verb, Some(prepared.verb));
            thread::yield_now();
        }
    });
    barrier.wait();
    writer.join().unwrap();
    reader.join().unwrap();
}

#[test]
fn captures_are_append_only_and_can_reference_exact_drafts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut store = Store::open(&path).unwrap();
    let draft = store
        .create_draft(&DraftCreate {
            id: Some("draft".into()),
            content: "posted".into(),
            context: None,
            verb: None,
            author_kind: AuthorKind::Human,
            provenance: json!({}),
        })
        .unwrap();
    let capture = store
        .record_capture(&CaptureInput {
            id: Some("capture".into()),
            observation: CaptureObservation::SubmitAttempt,
            parent_id: None,
            surface: "github-pr".into(),
            url: "https://github.com/o/r/pull/1".into(),
            content: "posted".into(),
            draft: Some(prose::DraftTarget {
                draft_id: draft.draft_id,
                version: draft.version,
            }),
            metadata: json!({"source":"test"}),
        })
        .unwrap();
    assert_eq!(capture.draft.unwrap().version, 1);
    assert_eq!(capture.observation, CaptureObservation::SubmitAttempt);
    assert_eq!(capture.parent_id, None);
    let retried = store
        .record_capture(&CaptureInput {
            id: Some("capture".into()),
            observation: CaptureObservation::SubmitAttempt,
            parent_id: None,
            surface: "github-pr".into(),
            url: "https://github.com/o/r/pull/1".into(),
            content: "posted".into(),
            draft: Some(prose::DraftTarget {
                draft_id: "draft".into(),
                version: 1,
            }),
            metadata: json!({"source":"test"}),
        })
        .unwrap();
    assert_eq!(retried.id, "capture");
    assert_eq!(store.list_captures(Some("github-pr")).unwrap().len(), 1);
    let inspection = rusqlite::Connection::open(path).unwrap();
    assert!(
        inspection
            .execute("DELETE FROM capture_events", [])
            .is_err()
    );
}

#[test]
fn schema_v1_migrates_transactionally_to_capture_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    drop(Store::open(&path).unwrap());
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER captures_no_update;
         DROP TRIGGER captures_no_delete;
         DROP TABLE capture_events;
         DELETE FROM schema_migrations WHERE version=2;
         PRAGMA user_version=1;",
        )
        .unwrap();
    drop(connection);
    let store = Store::open(path).unwrap();
    assert_eq!(store.schema_version().unwrap(), 3);
    assert!(store.list_captures(None).unwrap().is_empty());
}

#[test]
fn extension_is_mechanism_only_and_targets_both_surfaces() {
    for raw in [
        include_str!("../extension/manifests/chrome.json"),
        include_str!("../extension/manifests/firefox.json"),
    ] {
        let manifest: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(manifest["manifest_version"], 3);
        let matches = manifest["content_scripts"][0]["matches"]
            .as_array()
            .unwrap();
        assert!(matches.iter().any(|value| value == "https://github.com/*"));
        assert!(matches.iter().any(|value| value == "https://linear.app/*"));
        assert!(manifest.get("version").is_none());
    }
    let source = format!(
        "{}\n{}\n{}",
        include_str!("../extension/background.js"),
        include_str!("../extension/content.js"),
        include_str!("../extension/content-core.js")
    );
    assert!(source.contains("github-pr"));
    assert!(source.contains("submit-attempt"));
    assert!(source.contains("storage.local"));
    assert!(source.contains("location.origin"));
    assert!(!source.contains("Be concise"));
    assert!(!source.contains("tighten"));
}

#[tokio::test]
async fn serve_keeps_capture_available_without_a_backend() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let port = port();
    let task = tokio::spawn(run(ServeConfig {
        store_path: path.clone(),
        host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port,
        agent_command: None,
        agent_args: vec![],
        agent_timeout: Duration::from_secs(1),
    }));
    wait(port).await;
    let (status, health) = request(port, "GET", "/v1/health", None).await;
    assert_eq!(status, 200);
    assert_eq!(health["ready"], true);
    assert_eq!(health["store"]["ready"], true);
    assert_eq!(health["service"]["ready"], true);
    assert_eq!(health["backend"]["configured"], false);
    assert_eq!(health["backend"]["last_diagnostic"], Value::Null);
    let (status, recorded) = request(port, "POST", "/v1/capture", Some(&json!({"action":"record","capture":{
        "id":null,"surface":"linear","url":"https://linear.app/x","content":"posted","draft":null,"metadata":{}
    }}))).await;
    assert_eq!(status, 200);
    assert_eq!(recorded["item"]["observation"], "submit-attempt");
    assert_eq!(recorded["item"]["parent_id"], Value::Null);
    assert_eq!(
        Store::open(path)
            .unwrap()
            .list_captures(None)
            .unwrap()
            .len(),
        1
    );
    task.abort();
}

#[tokio::test]
async fn assist_assembles_material_calls_backend_and_records_versions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    seeded(&path);
    let initial = Store::open(&path)
        .unwrap()
        .create_draft(&DraftCreate {
            id: Some("supplied".into()),
            content: "earlier text".into(),
            context: Some(content_ref("code-review")),
            verb: Some(content_ref("tighten")),
            author_kind: AuthorKind::Human,
            provenance: json!({}),
        })
        .unwrap();
    let port = port();
    let task = tokio::spawn(run(ServeConfig {
        store_path: path.clone(),
        host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port,
        agent_command: Some("/bin/sh".into()),
        agent_args: vec![
            "-c".into(),
            "cat >/dev/null; printf 'candidate text'".into(),
        ],
        agent_timeout: Duration::from_secs(1),
    }));
    wait(port).await;
    let (status, result) = request(
        port,
        "POST",
        "/v1/assist",
        Some(&json!({
            "surface":"github-pr","context":"code-review","verb":"tighten","pack":"test",
            "selection":"wordy text","surrounding_context":"review body","draft":{
                "draft_id":initial.draft_id,"version":initial.version
            }
        })),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["candidate"], "candidate text");
    let versions = Store::open(path)
        .unwrap()
        .list_draft_versions(result["draft"]["draft_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(versions.len(), 3);
    assert_eq!(versions[0].author_kind, AuthorKind::Human);
    assert_eq!(versions[1].author_kind, AuthorKind::Capture);
    assert_eq!(versions[1].content, "wordy text");
    assert_eq!(versions[1].parent_version, Some(1));
    assert_eq!(versions[2].author_kind, AuthorKind::Agent);
    assert_eq!(versions[2].parent_version, Some(2));
    task.abort();
}

#[tokio::test]
async fn serve_refuses_non_loopback_binding() {
    let dir = tempfile::tempdir().unwrap();
    let error = run(ServeConfig {
        store_path: dir.path().join("store.db"),
        host: "0.0.0.0".parse().unwrap(),
        port: port(),
        agent_command: None,
        agent_args: vec![],
        agent_timeout: Duration::from_secs(1),
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("loopback"));
}

#[tokio::test]
async fn browser_router_denies_non_extension_operations_and_cors() {
    let dir = tempfile::tempdir().unwrap();
    let port = port();
    let task = tokio::spawn(run(ServeConfig {
        store_path: dir.path().join("store.db"),
        host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port,
        agent_command: None,
        agent_args: vec![],
        agent_timeout: Duration::from_secs(1),
    }));
    wait(port).await;

    for route in ["pack", "context", "draft", "attestation", "render"] {
        let (status, _, _) =
            request_with_headers(port, "POST", &format!("/v1/{route}"), Some(&json!({})), &[])
                .await;
        assert_eq!(status, 404, "route /v1/{route} must not be browser-facing");
    }
    let (status, _, _) = request_with_headers(
        port,
        "POST",
        "/v1/verb",
        Some(&json!({"action":"get","name":"secret","pack":null})),
        &[],
    )
    .await;
    assert_eq!(status, 422);
    let (status, _, _) = request_with_headers(
        port,
        "POST",
        "/v1/capture",
        Some(&json!({"action":"list","surface":null})),
        &[],
    )
    .await;
    assert_eq!(status, 422);

    let (status, headers, _) = request_with_headers(
        port,
        "OPTIONS",
        "/v1/assist",
        None,
        &[
            ("Origin", "https://evil.example"),
            ("Access-Control-Request-Method", "POST"),
        ],
    )
    .await;
    assert_eq!(status, 405);
    assert!(
        !headers
            .to_ascii_lowercase()
            .contains("access-control-allow")
    );
    let (_, headers, _) = request_with_headers(
        port,
        "GET",
        "/v1/health",
        None,
        &[("Origin", "https://evil.example")],
    )
    .await;
    assert!(
        !headers
            .to_ascii_lowercase()
            .contains("access-control-allow")
    );
    let (_, headers, _) = request_with_headers(
        port,
        "GET",
        "/v1/health",
        None,
        &[("Origin", "chrome-extension://arbitrary-extension-id")],
    )
    .await;
    assert!(
        !headers
            .to_ascii_lowercase()
            .contains("access-control-allow")
    );
    task.abort();
}

#[tokio::test]
async fn health_tracks_store_readiness_and_sanitized_backend_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    std::fs::create_dir(&path).unwrap();
    let port = port();
    let task = tokio::spawn(run(ServeConfig {
        store_path: path.clone(),
        host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port,
        agent_command: Some("/bin/sh".into()),
        agent_args: vec![
            "-c".into(),
            "cat >/dev/null; printf 'DO-NOT-LEAK' >&2; exit 9".into(),
        ],
        agent_timeout: Duration::from_secs(1),
    }));
    wait(port).await;
    let (status, health) = request(port, "GET", "/v1/health", None).await;
    assert_eq!(status, 503);
    assert_eq!(health["service"]["ready"], true);
    assert_eq!(health["store"]["ready"], false);
    assert_eq!(health["store"]["category"], "io-error");
    assert_eq!(health["backend"]["configured"], true);

    std::fs::remove_dir(&path).unwrap();
    seeded(&path);
    let supplied = Store::open(&path)
        .unwrap()
        .create_draft(&DraftCreate {
            id: Some("failed-assist".into()),
            content: "old source".into(),
            context: Some(content_ref("code-review")),
            verb: Some(content_ref("tighten")),
            author_kind: AuthorKind::Human,
            provenance: json!({}),
        })
        .unwrap();
    let (status, health) = request(port, "GET", "/v1/health", None).await;
    assert_eq!(status, 200);
    assert_eq!(health["ready"], true);

    let (status, failure) = request(
        port,
        "POST",
        "/v1/assist",
        Some(&json!({
            "surface":"github-pr","context":"code-review","verb":"tighten","pack":"test",
            "selection":"source","surrounding_context":"private surrounding","draft":{
                "draft_id":supplied.draft_id,"version":supplied.version
            }
        })),
    )
    .await;
    assert_eq!(status, 503);
    assert!(!failure.to_string().contains("DO-NOT-LEAK"));
    let versions = Store::open(&path)
        .unwrap()
        .list_draft_versions("failed-assist")
        .unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[1].author_kind, AuthorKind::Capture);
    assert_eq!(versions[1].content, "source");
    let (_, health) = request(port, "GET", "/v1/health", None).await;
    let diagnostic = &health["backend"]["last_diagnostic"];
    assert_eq!(diagnostic["outcome"], "failure");
    assert_eq!(diagnostic["category"], "exit");
    assert!(diagnostic["at_unix_ms"].as_u64().is_some());
    let text = diagnostic.to_string();
    assert!(!text.contains("DO-NOT-LEAK"));
    assert!(!text.contains("private surrounding"));
    task.abort();
}

#[tokio::test]
async fn graceful_shutdown_drains_an_in_flight_assist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    seeded(&path);
    let port = port();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(run_with_shutdown(
        ServeConfig {
            store_path: path,
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
            agent_command: Some("/bin/sh".into()),
            agent_args: vec![
                "-c".into(),
                "cat >/dev/null; sleep 0.15; printf drained".into(),
            ],
            agent_timeout: Duration::from_secs(2),
        },
        async move {
            let _ = shutdown_rx.await;
        },
    ));
    wait(port).await;
    let response = tokio::spawn(async move {
        request(
            port,
            "POST",
            "/v1/assist",
            Some(&json!({
                "surface":"github-pr","context":"code-review","verb":"tighten","pack":"test",
                "selection":"source","surrounding_context":"","draft":null
            })),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    shutdown_tx.send(()).unwrap();
    let (status, body) = response.await.unwrap();
    assert_eq!(status, 200);
    assert_eq!(body["candidate"], "drained");
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("server did not finish graceful shutdown")
        .unwrap()
        .unwrap();
}
