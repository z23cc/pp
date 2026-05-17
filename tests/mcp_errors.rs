mod common;

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
#[ignore = "expensive smoke test: generates and builds a wrapper CLI; run with `cargo test -- --ignored`"]
fn mcp_error_shapes_are_distinguishable() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/items/42")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_header("x-error", "nope")
        .with_body(r#"{"message":"unauthorized"}"#)
        .create();

    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "mcp-errors.yaml",
        &format!(
            r#"
openapi: 3.0.0
info:
  title: Error API
  version: "1.0.0"
servers:
  - url: {}
security:
  - apiKeyAuth: []
paths:
  /items/{{id}}:
    get:
      operationId: getItem
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: integer
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  ok:
                    type: boolean
components:
  securitySchemes:
    apiKeyAuth:
      type: apiKey
      in: header
      name: X-API-Key
"#,
            server.url()
        ),
    );
    let out_dir = temp.path().join("out");
    let output = common::pp_generate_command(&spec, &out_dir)
        .arg("--build")
        .output()
        .expect("failed to run pp generate");
    common::assert_success(output, "pp generate --build");
    let bin = common::generated_bin(&out_dir, "error-api");

    let invalid_params = call_tool(&bin, Some(("ERROR_API_API_KEY", "secret")), json!({}));
    assert_eq!(invalid_params["error"]["code"], -32602);

    let auth_missing = call_tool(&bin, None, json!({ "id": 42 }));
    assert_eq!(auth_missing["result"]["isError"], true);
    assert_eq!(
        auth_missing["result"]["structuredContent"]["kind"],
        "auth_missing"
    );
    assert_eq!(
        auth_missing["result"]["structuredContent"]["env"],
        "ERROR_API_API_KEY"
    );

    let upstream = call_tool(
        &bin,
        Some(("ERROR_API_API_KEY", "secret")),
        json!({ "id": 42 }),
    );
    assert_eq!(upstream["result"]["isError"], true);
    assert_eq!(
        upstream["result"]["structuredContent"]["kind"],
        "upstream_http_error"
    );
    assert_eq!(upstream["result"]["structuredContent"]["status"], 401);
    assert!(upstream["result"]["structuredContent"]["headers"].is_object());
    mock.assert();
}

#[test]
#[ignore = "expensive smoke test: generates and builds a wrapper CLI; run with `cargo test -- --ignored`"]
fn mcp_transport_error_is_tool_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "transport.yaml",
        r#"
openapi: 3.0.0
info:
  title: Transport API
  version: "1.0.0"
servers:
  - url: http://127.0.0.1:9
paths:
  /ping:
    get:
      operationId: ping
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  ok:
                    type: boolean
"#,
    );
    let out_dir = temp.path().join("out");
    let output = common::pp_generate_command(&spec, &out_dir)
        .arg("--build")
        .output()
        .expect("failed to run pp generate");
    common::assert_success(output, "pp generate --build");
    let bin = common::generated_bin(&out_dir, "transport-api");

    let response = call_named_tool(&bin, "ping", None, json!({}));
    assert_eq!(response["result"]["isError"], true);
    assert_eq!(
        response["result"]["structuredContent"]["kind"],
        "transport_error"
    );
}

fn call_tool(bin: &std::path::Path, env: Option<(&str, &str)>, arguments: Value) -> Value {
    call_named_tool(bin, "get_item", env, arguments)
}

fn call_named_tool(
    bin: &std::path::Path,
    tool_name: &str,
    env: Option<(&str, &str)>,
    arguments: Value,
) -> Value {
    let mut command = Command::new(bin);
    common::disable_proxy(&mut command);
    command
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some((name, value)) = env {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("spawn mcp");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));

    writeln!(
        stdin,
        "{}",
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}
        })
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": tool_name, "arguments": arguments}
        })
    )
    .unwrap();
    drop(stdin);

    let mut line = String::new();
    let mut response = Value::Null;
    while stdout.read_line(&mut line).expect("read mcp") > 0 {
        if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
            if value.get("id") == Some(&json!(2)) {
                response = value;
                break;
            }
        }
        line.clear();
    }
    let _ = child.kill();
    let _ = child.wait();
    response
}
