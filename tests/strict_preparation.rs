mod common;

use serde_json::Value;
use std::process::{Command, Output};

const LOSSY_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Lossy Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /items:
    get:
      operationId: listItems
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
            application/xml:
              schema:
                type: object
        '404':
          description: missing
"#;

const MIXED_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Mixed Fixture
  version: "1.0.0"
paths:
  /clean:
    get:
      operationId: cleanOp
      responses:
        '200':
          description: ok
  /messy:
    get:
      operationId: messyOp
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
            application/xml:
              schema:
                type: object
        '404':
          description: missing
"#;

const RESPONSE_RELAXATION_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Response Relaxation Fixture
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                required: [name]
                properties:
                  name:
                    type: string
"#;

const NO_SERVER_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: No Server Fixture
  version: "1.0.0"
paths:
  /ping:
    get:
      operationId: ping
      responses:
        '200':
          description: ok
"#;

const COMPONENT_MULTIPART_ONLY_SPEC: &str = r##"
openapi: 3.0.0
info:
  title: Component Multipart Fixture
  version: "1.0.0"
paths:
  /files:
    post:
      operationId: uploadFile
      requestBody:
        $ref: "#/components/requestBodies/Upload"
      responses:
        '200':
          description: ok
components:
  requestBodies:
    Upload:
      content:
        multipart/form-data:
          schema:
            type: object
"##;

const REMOVED_APPROVAL_FLAGS: &[&[&str]] = &[
    &["--allow-", "effect"],
    &["--allow-", "report-code"],
    &["--allow-", "com", "pat-", "normal", "ization"],
];

const AMBIGUOUS_AUTH_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Ambiguous Auth Fixture
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      responses:
        '200':
          description: ok
components:
  securitySchemes:
    apiKeyAuth:
      type: apiKey
      in: header
      name: X-API-Key
    bearerAuth:
      type: http
      scheme: bearer
"#;

const MISSING_OPERATION_ID_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Missing OperationId Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /items/{id}:
    patch:
      responses:
        '200':
          description: ok
"#;

#[test]
fn inspect_preserves_typed_openapi_shapes() {
    let output = run_inspect(&[]);
    assert!(
        output.status.success(),
        "strict inspect failed\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("spec.prepare."), "stderr:\n{stderr}");
}

#[test]
fn list_operations_supports_discovery_without_opt_in() {
    let output = run_inspect(&["--list-operations"]);
    assert!(
        output.status.success(),
        "list operations should be a discovery path\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("listItems"), "stdout:\n{stdout}");
}

#[test]
fn list_operations_skips_auth_derivation_for_discovery() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "ambiguous-auth.yaml", AMBIGUOUS_AUTH_SPEC);

    let default_output = Command::new(common::pp_bin())
        .arg("inspect")
        .arg(&spec)
        .output()
        .expect("failed to run pp inspect");
    assert!(!default_output.status.success());
    let stderr = String::from_utf8_lossy(&default_output.stderr);
    assert!(stderr.contains("ambiguous auth schemes: apiKeyAuth, bearerAuth"));

    let listing_output = Command::new(common::pp_bin())
        .arg("inspect")
        .arg(spec)
        .arg("--list-operations")
        .output()
        .expect("failed to run pp inspect --list-operations");
    assert!(listing_output.status.success());
    let stdout = String::from_utf8_lossy(&listing_output.stdout);
    assert!(stdout.contains("listItems"), "stdout:\n{stdout}");
}

#[test]
fn strict_slice_ignores_unselected_operations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "mixed.yaml", MIXED_SPEC);
    let output = Command::new(common::pp_bin())
        .arg("inspect")
        .arg(spec)
        .arg("--include-operation")
        .arg("cleanOp")
        .arg("--reports")
        .output()
        .expect("failed to run pp inspect");

    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).expect("inspect report JSON");
    let reports = value["reports"].as_array().expect("reports array");
    assert!(reports.iter().all(|report| report["code"]
        .as_str()
        .is_some_and(|code| code.starts_with("spec.slice."))));
}

#[test]
fn inspect_preserves_multipart_request_body() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "component-multipart.yaml",
        COMPONENT_MULTIPART_ONLY_SPEC,
    );
    let output = Command::new(common::pp_bin())
        .arg("inspect")
        .arg(spec)
        .arg("--reports")
        .output()
        .expect("failed to run pp inspect");

    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).expect("inspect report JSON");
    let reports = value["reports"].as_array().expect("reports array");
    assert!(reports.is_empty());
}

#[test]
fn inspect_preserves_response_schema() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "response-relaxation.yaml",
        RESPONSE_RELAXATION_SPEC,
    );
    let output = Command::new(common::pp_bin())
        .arg("inspect")
        .arg(spec)
        .arg("--reports")
        .output()
        .expect("failed to run pp inspect");

    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).expect("inspect report JSON");
    let reports = value["reports"].as_array().expect("reports array");
    assert!(reports.is_empty());
}

#[test]
fn generate_writes_transform_plan_without_spec_shape_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "simple.yaml", NO_SERVER_SPEC);
    let out_dir = temp.path().join("out");
    let output = Command::new(common::pp_bin())
        .arg("generate")
        .arg(spec)
        .arg("-o")
        .arg(&out_dir)
        .arg("--base-url")
        .arg("https://example.test")
        .output()
        .expect("failed to run pp generate");

    common::assert_success(output, "pp generate --base-url");
    let plan_path = out_dir.join("pp-transform-plan.json");
    let value: Value = serde_json::from_slice(
        &std::fs::read(&plan_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", plan_path.display())),
    )
    .expect("transform plan JSON");
    let entries = value["entries"].as_array().expect("entries array");
    assert!(entries.is_empty());
}

#[test]
fn generate_rejects_missing_explicit_operation_id_with_exclude_hint() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "missing-operation-id.yaml",
        MISSING_OPERATION_ID_SPEC,
    );
    let output = Command::new(common::pp_bin())
        .arg("generate")
        .arg(spec)
        .arg("-o")
        .arg(temp.path().join("out"))
        .output()
        .expect("failed to run pp generate");

    assert!(!output.status.success(), "generate unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("operation PATCH /items/{id} is missing operationId"));
    assert!(stderr.contains("explicit operationId is required for codegen/MCP identity"));
    assert!(stderr.contains("--exclude-operation \"patch /items/{id}\""));
}

#[test]
fn removed_transform_approval_flags_are_unknown_arguments() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "simple.yaml", NO_SERVER_SPEC);

    for parts in REMOVED_APPROVAL_FLAGS {
        let flag = parts.concat();
        let inspect_output = Command::new(common::pp_bin())
            .arg("inspect")
            .arg(&spec)
            .arg(&flag)
            .output()
            .expect("failed to run pp inspect");
        assert!(
            !inspect_output.status.success(),
            "{flag} unexpectedly accepted by inspect"
        );
        assert_unknown_argument(&inspect_output, &flag);

        let generate_output = Command::new(common::pp_bin())
            .arg("generate")
            .arg(&spec)
            .arg("-o")
            .arg(temp.path().join("out"))
            .arg(&flag)
            .output()
            .expect("failed to run pp generate");
        assert!(
            !generate_output.status.success(),
            "{flag} unexpectedly accepted by generate"
        );
        assert_unknown_argument(&generate_output, &flag);
    }
}

#[test]
fn generate_rejects_missing_server_without_explicit_base_url() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "no-server.yaml", NO_SERVER_SPEC);
    let output = Command::new(common::pp_bin())
        .arg("generate")
        .arg(spec)
        .arg("-o")
        .arg(temp.path().join("out"))
        .output()
        .expect("failed to run pp generate");

    assert!(!output.status.success(), "generate unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no servers[0].url") && stderr.contains("--base-url"));
}

fn assert_unknown_argument(output: &Output, flag: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unexpected argument") && stderr.contains(flag),
        "stderr:\n{stderr}"
    );
}

fn run_inspect(args: &[&str]) -> Output {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "lossy.yaml", LOSSY_SPEC);
    Command::new(common::pp_bin())
        .arg("inspect")
        .arg(spec)
        .args(args)
        .output()
        .expect("failed to run pp inspect")
}
