mod common;

use common::{assert_success, generated_bin, pp_generate_command, run_pp_generate, write_spec};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

const OPENAPI_31_SUBSET: &str = r##"
openapi: 3.1.0
info: { title: OpenAPI 31 Subset API, version: '1.0' }
servers:
  - url: https://api.example.test
paths:
  /items/{id}:
    post:
      operationId: updateItem
      parameters:
        - name: id
          in: path
          required: true
          schema: { type: string }
        - name: tags
          in: query
          explode: true
          schema:
            type: array
            items: { type: string }
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/UpdateItem'
      responses:
        '200': { description: ok }
components:
  schemas:
    UpdateItem:
      type: object
      required: [name]
      properties:
        name:
          type: [string, 'null']
        rating:
          $ref: '#/$defs/Rating'
      $defs:
        Rating:
          type: integer
"##;

#[test]
fn generate_accepts_openapi31_safe_subset() {
    let temp = tempfile::tempdir().unwrap();
    let spec = write_spec(temp.path(), "openapi31.yaml", OPENAPI_31_SUBSET);
    let out = temp.path().join("generated");

    let output = pp_generate_command(&spec, &out)
        .output()
        .expect("run pp generate");
    assert_success(output, "openapi 3.1 subset generate");

    let cli_builder = std::fs::read_to_string(out.join("src/cli_builder.rs")).unwrap();
    assert!(cli_builder.contains("update_item"));
}

#[test]
#[ignore = "expensive smoke test: generates, builds, and runs a wrapper CLI from an OpenAPI 3.1 safe-subset spec; run with `cargo test --test openapi31 -- --ignored`"]
fn openapi31_safe_subset_generated_workspace_builds_and_runs() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/items/item-1?tags=a&tags=b")
        .expect(2)
        .match_body(mockito::Matcher::Json(json!({ "name": null, "rating": 5 })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true}"#)
        .create();

    let temp = tempfile::tempdir().unwrap();
    let spec_body = OPENAPI_31_SUBSET.replace("https://api.example.test", &server.url());
    let spec = write_spec(temp.path(), "openapi31-smoke.yaml", &spec_body);
    let out = temp.path().join("generated");

    assert_success(
        run_pp_generate(&spec, &out),
        "pp generate --build openapi 3.1 subset",
    );

    let mut command = Command::new(generated_bin(&out, "subset-api"));
    let output = common::disable_proxy(&mut command)
        .arg("update_item")
        .arg("--id")
        .arg("item-1")
        .arg("--tags")
        .arg("a")
        .arg("--tags")
        .arg("b")
        .arg("--name")
        .arg("null")
        .arg("--rating")
        .arg("5")
        .arg("--json")
        .output()
        .expect("run generated OpenAPI 3.1 command");
    assert_success(output, "generated OpenAPI 3.1 update_item command");

    let mcp = call_tool(
        &generated_bin(&out, "subset-api"),
        "update_item",
        json!({"id":"item-1","tags":["a","b"],"name":null,"rating":5}),
    );
    assert_eq!(mcp["result"]["structuredContent"]["ok"], true);
    mock.assert();
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

#[test]
fn generate_rejects_nullable_query_array_items() {
    let temp = tempfile::tempdir().unwrap();
    let spec = write_spec(
        temp.path(),
        "nullable-array-items.yaml",
        r#"
openapi: 3.1.0
info: { title: Nullable Array Items API, version: '1.0' }
servers:
  - url: https://api.example.test
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: tags
          in: query
          explode: true
          schema:
            type: array
            items:
              type: [string, 'null']
      responses:
        '200': { description: ok }
"#,
    );
    let out = temp.path().join("generated");

    let output = pp_generate_command(&spec, &out)
        .output()
        .expect("run pp generate");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nullable array items for parameter 'tags'"),
        "{stderr}"
    );
}

#[test]
fn generate_rejects_unsupported_openapi31_json_schema_feature() {
    let temp = tempfile::tempdir().unwrap();
    let spec = write_spec(
        temp.path(),
        "unsupported31.yaml",
        r#"
openapi: 3.1.0
info: { title: Unsupported 31 API, version: '1.0' }
servers:
  - url: https://api.example.test
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: filter
          in: query
          schema:
            oneOf:
              - type: string
              - type: integer
      responses:
        '200': { description: ok }
"#,
    );
    let out = temp.path().join("generated");

    let output = pp_generate_command(&spec, &out)
        .output()
        .expect("run pp generate");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unsupported JSON Schema feature 'oneOf'"),
        "{stderr}"
    );
    assert!(stderr.contains("parameter 'filter'"), "{stderr}");
}
