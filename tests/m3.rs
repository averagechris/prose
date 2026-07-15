use prose::{
    AuthorKind, CaptureInput, ContextSpec, DraftCreate, PackDocument, Store, VerbConstraints,
    VerbFamily, VerbSpec,
    server::{ServeConfig, run},
};
use serde_json::{Value, json};
use std::{
    net::{IpAddr, Ipv4Addr, TcpListener},
    path::Path,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn port() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.local_addr().unwrap().port()
}

async fn request(port: u16, method: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
    let body = body.map(Value::to_string).unwrap_or_default();
    let mut stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let wire = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(wire.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(body).unwrap())
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
    let retried = store
        .record_capture(&CaptureInput {
            id: Some("capture".into()),
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
    assert_eq!(store.schema_version().unwrap(), 2);
    assert!(store.list_captures(None).unwrap().is_empty());
}

#[test]
fn extension_is_mechanism_only_and_targets_both_surfaces() {
    let manifest: Value = serde_json::from_str(include_str!("../extension/manifest.json")).unwrap();
    assert_eq!(manifest["manifest_version"], 3);
    let matches = manifest["content_scripts"][0]["matches"]
        .as_array()
        .unwrap();
    assert!(matches.iter().any(|value| value == "https://github.com/*"));
    assert!(matches.iter().any(|value| value == "https://linear.app/*"));
    let source = format!(
        "{}\n{}",
        include_str!("../extension/background.js"),
        include_str!("../extension/content.js")
    );
    assert!(source.contains("github-pr"));
    assert!(source.contains("browser-submit-event"));
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
    assert_eq!(health["backend_available"], false);
    let (status, _) = request(port, "POST", "/v1/capture", Some(&json!({"action":"record","capture":{
        "id":null,"surface":"linear","url":"https://linear.app/x","content":"posted","draft":null,"metadata":{}
    }}))).await;
    assert_eq!(status, 200);
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
            "selection":"wordy text","surrounding_context":"review body","draft":null
        })),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["candidate"], "candidate text");
    let versions = Store::open(path)
        .unwrap()
        .list_draft_versions(result["draft"]["draft_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].author_kind, AuthorKind::Capture);
    assert_eq!(versions[1].author_kind, AuthorKind::Agent);
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
