mod common;

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn generate_rejects_reserved_pp_query_parameter() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "reserved.yaml",
        r#"
openapi: 3.0.0
info:
  title: Reserved API
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: _pp_fields
          in: query
          schema:
            type: string
      responses:
        '200':
          description: ok
"#,
    );
    let output = Command::new(common::pp_bin())
        .arg("generate")
        .arg(&spec)
        .arg("-o")
        .arg(temp.path().join("out"))
        .output()
        .expect("run pp generate");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("reserved pp namespace"),
        "stderr:\n{stderr}"
    );
    assert!(stderr.contains("_pp_fields"), "stderr:\n{stderr}");
}

#[test]
#[ignore = "expensive smoke test: generates and builds a wrapper CLI; run with `cargo test --test mcp_usability -- --ignored`"]
fn json_object_request_body_spec_generates_and_builds() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "json-body.yaml",
        r#"
openapi: 3.0.0
info:
  title: Json Body API
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /items:
    post:
      operationId: createItem
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              properties:
                name:
                  type: string
              required: [name]
      responses:
        '200':
          description: ok
"#,
    );
    let out_dir = temp.path().join("out");
    common::assert_success(
        common::run_pp_generate(&spec, &out_dir),
        "pp generate --build json body",
    );
    assert!(common::generated_bin(&out_dir, "json-body-api").exists());
}

#[test]
#[ignore = "expensive smoke test: generates and builds a wrapper CLI; run with `cargo test --test mcp_usability -- --ignored`"]
fn query_array_spec_generates_builds_and_sends_repeated_query_params() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/items?tags=a&tags=b")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true}"#)
        .create();

    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "query-array.yaml",
        &format!(
            r#"
openapi: 3.0.0
info:
  title: Query Array API
  version: "1.0.0"
servers:
  - url: {}
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: tags
          in: query
          required: false
          schema:
            type: array
            items:
              type: string
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
            server.url()
        ),
    );
    let out_dir = temp.path().join("out");
    common::assert_success(
        common::run_pp_generate(&spec, &out_dir),
        "pp generate --build query array",
    );

    let mut command = Command::new(common::generated_bin(&out_dir, "query-array-api"));
    let output = common::disable_proxy(&mut command)
        .arg("list_items")
        .arg("--tags")
        .arg("a")
        .arg("--tags")
        .arg("b")
        .arg("--json")
        .output()
        .expect("failed to run generated query-array command");
    common::assert_success(output, "generated list_items with repeated query params");
    mock.assert();
}

#[test]
#[ignore = "expensive smoke test: generates and builds a wrapper CLI; run with `cargo test --test mcp_usability -- --ignored`"]
fn tools_list_uses_cursor_pagination() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "many-tools.yaml", &many_tools_spec(105));
    let out_dir = temp.path().join("out");
    common::assert_success(
        common::run_pp_generate(&spec, &out_dir),
        "pp generate --build",
    );
    let bin = common::generated_bin(&out_dir, "many-tools-api");

    let first = list_tools(&bin, None);
    assert_eq!(first["result"]["tools"].as_array().unwrap().len(), 100);
    let cursor = first["result"]["nextCursor"].as_str().expect("next cursor");

    let second = list_tools(&bin, Some(cursor));
    assert_eq!(second["result"]["tools"].as_array().unwrap().len(), 5);
    assert!(second["result"].get("nextCursor").is_none());
}

#[test]
#[ignore = "expensive smoke test: generates and builds a wrapper CLI; run with `cargo test --test mcp_usability -- --ignored`"]
fn mcp_response_shaping_is_opt_in_and_success_only() {
    let mut server = mockito::Server::new();
    let ok = server
        .mock("GET", "/item")
        .expect(2)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"bulbasaur","types":[{"slot":1}],"stats":[],"empty":{},"nil":null,"nested":{"keep":"yes","drop":null}}"#)
        .create();
    let fail = server
        .mock("GET", "/fail")
        .with_status(500)
        .with_header("content-type", "application/json")
        .with_body(r#"{"message":"boom","nil":null}"#)
        .create();

    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "shape.yaml", &shape_spec(&server.url()));
    let out_dir = temp.path().join("out");
    let output = common::pp_generate_command(&spec, &out_dir)
        .arg("--build")
        .output()
        .expect("failed to run pp generate");
    common::assert_success(output, "pp generate --build");
    let bin = common::generated_bin(&out_dir, "shape-api");

    let full = call_tool(&bin, "get_item", json!({}));
    assert_eq!(full["result"]["structuredContent"]["name"], "bulbasaur");
    assert!(full["result"]["structuredContent"].get("stats").is_some());

    let shaped = call_tool(
        &bin,
        "get_item",
        json!({"_pp_fields":["name","nested.keep","stats"],"_pp_compact":true}),
    );
    assert_eq!(
        shaped["result"]["structuredContent"],
        json!({"name":"bulbasaur","nested":{"keep":"yes"}})
    );

    let error = call_tool(
        &bin,
        "fail_item",
        json!({"_pp_fields":["message"],"_pp_compact":true}),
    );
    assert_eq!(error["result"]["isError"], true);
    assert_eq!(
        error["result"]["structuredContent"]["kind"],
        "upstream_http_error"
    );
    assert!(error["result"]["structuredContent"].get("body").is_some());

    ok.assert();
    fail.assert();
}

fn list_tools(bin: &std::path::Path, cursor: Option<&str>) -> Value {
    let params = cursor
        .map(|cursor| json!({"cursor": cursor}))
        .unwrap_or_else(|| json!({}));
    mcp_request(
        bin,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":params}),
    )
}

fn call_tool(bin: &std::path::Path, tool_name: &str, arguments: Value) -> Value {
    mcp_request(
        bin,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":tool_name,"arguments":arguments}}),
    )
}

fn mcp_request(bin: &std::path::Path, request: Value) -> Value {
    let mut command = Command::new(bin);
    common::disable_proxy(&mut command);
    command
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut child = command.spawn().expect("spawn mcp");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));

    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}})
    )
    .unwrap();
    writeln!(stdin, "{}", request).unwrap();
    drop(stdin);

    let mut line = String::new();
    while stdout.read_line(&mut line).expect("read mcp") > 0 {
        if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
            if value.get("id") == Some(&json!(2)) {
                let _ = child.kill();
                let _ = child.wait();
                return value;
            }
        }
        line.clear();
    }
    let _ = child.kill();
    let _ = child.wait();
    Value::Null
}

fn many_tools_spec(count: usize) -> String {
    let mut paths = String::new();
    for index in 0..count {
        paths.push_str(&format!(
            r#"
  /items/{index}:
    get:
      operationId: getItem{index}
      responses:
        '200':
          description: ok
"#
        ));
    }
    format!(
        r#"openapi: 3.0.0
info:
  title: Many Tools API
  version: "1.0.0"
servers:
  - url: https://example.test
paths:{paths}
"#
    )
}

fn shape_spec(base_url: &str) -> String {
    format!(
        r#"openapi: 3.0.0
info:
  title: Shape API
  version: "1.0.0"
servers:
  - url: {base_url}
paths:
  /item:
    get:
      operationId: getItem
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  name:
                    type: string
                  types:
                    type: array
                    items:
                      type: object
                  stats:
                    type: array
                    items:
                      type: object
                  nested:
                    type: object
                    properties:
                      keep:
                        type: string
  /fail:
    get:
      operationId: failItem
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  message:
                    type: string
"#
    )
}
