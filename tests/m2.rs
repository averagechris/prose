use prose::{Application, ContextRequest, ContextSpec, PackDocument, Store};
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
        audience_tiers: vec![],
        surface_mappings: vec![],
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
