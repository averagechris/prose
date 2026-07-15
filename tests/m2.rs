use prose::{
    Application, AudienceTier, AuthorKind, ContextRequest, ContextSpec, PackDocument, Store,
    SurfaceMapping,
};
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn pack() -> PackDocument {
    PackDocument {
        id: "test".into(),
        description: "test material".into(),
        contexts: vec![ContextSpec {
            name: "code-review".into(),
            description: "review voice".into(),
            content: "Be direct.".into(),
        }],
        verbs: vec![],
        audience_tiers: vec![AudienceTier {
            name: "team".into(),
            description: "team".into(),
            requirements: vec![],
        }],
        surface_mappings: vec![SurfaceMapping {
            surface: "github".into(),
            context: "code-review".into(),
            default_audience_tier: "team".into(),
        }],
    }
}

fn mcp(path: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_prose"))
        .args(["--store", path.to_str().unwrap(), "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for request in requests {
            writeln!(stdin, "{request}").unwrap();
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},
        "clientInfo":{"name":"prose-test","version":"1"}
    }})
}

fn response_by_id(responses: &[Value], id: u64) -> &Value {
    responses
        .iter()
        .find(|response| response["id"] == id)
        .unwrap()
}

#[test]
fn mcp_lists_all_transport_neutral_capability_groups() {
    let dir = tempfile::tempdir().unwrap();
    let responses = mcp(
        &dir.path().join("store.db"),
        &[
            initialize(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        ],
    );
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "prose");
    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "attestation",
            "capture",
            "context",
            "draft",
            "pack",
            "render",
            "surface",
            "verb"
        ]
    );
    assert!(
        tools
            .iter()
            .all(|tool| tool["inputSchema"]["type"] == "object")
    );
}

#[test]
fn mcp_records_capture_observation_and_parent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    mcp(
        &path,
        &[
            initialize(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"capture","arguments":{"action":"record","capture":{
                "id":"attempt","observation":"submit-attempt","parent_id":null,"surface":"linear","url":"https://linear.app/acme/issue/ONE-1","content":"attempted","draft":null,"metadata":{}
            }}}}),
        ],
    );
    let responses = mcp(
        &path,
        &[
            initialize(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"capture","arguments":{"action":"record","capture":{
                "id":"confirmed","observation":"surface-confirmed-post","parent_id":"attempt","surface":"linear","url":"https://linear.app/acme/issue/ONE-1?posted=true","content":"confirmed exactly","draft":null,"metadata":{}
            }}}}),
        ],
    );
    let response = response_by_id(&responses, 3);
    let result: Value = serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("unexpected MCP response: {response}")),
    )
    .unwrap();
    assert_eq!(result["item"]["observation"], "surface-confirmed-post");
    assert_eq!(result["item"]["parent_id"], "attempt");
    assert_eq!(
        Store::open(path)
            .unwrap()
            .list_captures(None)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn cli_and_mcp_context_use_the_same_application_result() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let mut store = Store::open(&path).unwrap();
    store.create_pack(&pack()).unwrap();
    store.use_pack("test").unwrap();
    drop(store);

    let expected = Application::new(&path)
        .context(ContextRequest {
            name: "code-review".into(),
            pack: None,
        })
        .unwrap();
    let responses = mcp(
        &path,
        &[
            initialize(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"context","arguments":{"name":"code-review","pack":null}
            }}),
        ],
    );
    let text = responses[1]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(text).unwrap(), expected);
}

#[test]
fn mcp_pack_inspects_surface_origin_with_application_parity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    Store::open(&path).unwrap().create_pack(&pack()).unwrap();
    let expected = Application::new(&path)
        .pack(prose::PackRequest::ItemOrigin {
            pack_id: "test".into(),
            kind: prose::ItemKind::Surface,
            id: "github".into(),
            revision: Some(1),
        })
        .unwrap();
    let responses = mcp(
        &path,
        &[
            initialize(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"pack","arguments":{"action":"item-origin","pack_id":"test","kind":"surface","id":"github","revision":1}
            }}),
        ],
    );
    let response = response_by_id(&responses, 2);
    assert_ne!(response["result"]["isError"], true, "{response}");
    let actual: Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn mcp_draft_create_rejects_omitted_author_kind() {
    let dir = tempfile::tempdir().unwrap();
    let responses = mcp(
        &dir.path().join("store.db"),
        &[
            initialize(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"draft","arguments":{
                    "action":"create",
                    "draft":{"id":"missing-author","content":"body","context":null,"verb":null}
                }
            }}),
        ],
    );

    let response = response_by_id(&responses, 2);
    assert_eq!(response["result"]["isError"], true, "{responses:?}");
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("author_kind"),
        "{responses:?}"
    );
}

#[test]
fn mcp_draft_append_rejects_omitted_author_kind() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    Store::open(&path)
        .unwrap()
        .create_draft(&prose::DraftCreate {
            id: Some("append-author".into()),
            content: "first".into(),
            context: None,
            verb: None,
            author_kind: AuthorKind::Human,
            provenance: json!({}),
        })
        .unwrap();

    let responses = mcp(
        &path,
        &[
            initialize(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"draft","arguments":{
                    "action":"append","id":"append-author","parent":1,"content":"second",
                    "provenance":{"transport":"mcp"}
                }
            }}),
        ],
    );

    let response = response_by_id(&responses, 2);
    assert_eq!(response["result"]["isError"], true, "{responses:?}");
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("author_kind"),
        "{responses:?}"
    );
}

#[test]
fn mcp_draft_actions_accept_direct_arguments_and_store_explicit_agent_author() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let create_responses = mcp(
        &path,
        &[
            initialize(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"draft","arguments":{
                    "action":"create",
                    "draft":{
                        "id":"agent-draft","content":"first","context":null,"verb":null,
                        "author_kind":"agent","provenance":{"transport":"mcp"}
                    }
                }
            }}),
        ],
    );
    assert_eq!(
        response_by_id(&create_responses, 2)["result"]["isError"],
        false,
        "{create_responses:?}"
    );

    let append_responses = mcp(
        &path,
        &[
            initialize(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"draft","arguments":{
                    "action":"append","id":"agent-draft","parent":1,"content":"second",
                    "author_kind":"agent","provenance":{"transport":"mcp"}
                }
            }}),
        ],
    );

    assert_eq!(
        response_by_id(&append_responses, 2)["result"]["isError"],
        false,
        "{append_responses:?}"
    );
    let versions = Store::open(&path)
        .unwrap()
        .list_draft_versions("agent-draft")
        .unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].author_kind, AuthorKind::Agent);
    assert_eq!(versions[1].author_kind, AuthorKind::Agent);
}

#[test]
fn render_emits_only_a_live_pointer() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_prose"))
        .args([
            "--store",
            dir.path().join("store.db").to_str().unwrap(),
            "render",
            "--host",
            "opencode",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("fetch the current context module and verb specification"));
    assert!(text.contains("human explicitly approves"));
    assert!(!text.contains("Be concise"));
    assert!(text.lines().count() <= 10);
}
