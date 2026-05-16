mod common;

use serde_json::Value;
use std::process::{Command, Output};

const LOSSY_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Lossy Fixture
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

const RESPONSE_VARIANT_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Response Variant Fixture
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      responses:
        '200':
          description: ok
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
fn inspect_rejects_compat_normalization_by_default() {
    let output = run_inspect(&[]);
    assert!(
        !output.status.success(),
        "strict inspect unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("strict transform policy rejected"),
        "stderr did not explain strict rejection:\n{stderr}"
    );
    assert!(
        stderr.contains("spec.normalize.response_variants_pruned"),
        "stderr did not name response pruning report:\n{stderr}"
    );
    assert!(
        stderr.contains("--allow-compat-normalization"),
        "stderr did not include opt-in hint:\n{stderr}"
    );
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
fn strict_slice_ignores_lossy_reports_from_unselected_operations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "mixed.yaml", MIXED_SPEC);
    let output = Command::new(common::pp_bin())
        .arg("inspect")
        .arg(spec)
        .arg("--include-operation")
        .arg("cleanOp")
        .output()
        .expect("failed to run pp inspect");

    assert!(
        output.status.success(),
        "strict sliced inspect failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn strict_policy_rejects_component_multipart_only_request_body() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "component-multipart.yaml",
        COMPONENT_MULTIPART_ONLY_SPEC,
    );
    let output = Command::new(common::pp_bin())
        .arg("inspect")
        .arg(spec)
        .output()
        .expect("failed to run pp inspect");

    assert!(
        !output.status.success(),
        "strict inspect unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("strict transform policy rejected"),
        "stderr did not explain strict rejection:\n{stderr}"
    );
    assert!(
        stderr.contains("spec.normalize.unsupported_request_bodies_dropped"),
        "stderr did not name unsupported request-body report:\n{stderr}"
    );
    assert!(
        stderr.contains("component requestBody Upload"),
        "stderr did not identify component request body:\n{stderr}"
    );
}

#[test]
fn strict_policy_rejects_response_relaxation_before_mutation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "response-relaxation.yaml",
        RESPONSE_RELAXATION_SPEC,
    );
    let output = Command::new(common::pp_bin())
        .arg("inspect")
        .arg(spec)
        .output()
        .expect("failed to run pp inspect");

    assert!(
        !output.status.success(),
        "strict inspect unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("strict transform policy rejected"),
        "stderr did not explain strict rejection:\n{stderr}"
    );
    assert!(
        stderr.contains("spec.normalize.response_schemas_relaxed"),
        "stderr did not name response relaxation report:\n{stderr}"
    );
    assert!(
        stderr.contains("tolerant deserialization"),
        "stderr did not preserve response relaxation warning text:\n{stderr}"
    );
}

#[test]
fn inspect_allows_component_multipart_only_request_body_when_explicit() {
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
        .arg("--allow-compat-normalization")
        .output()
        .expect("failed to run pp inspect");

    common::assert_success(
        output,
        "pp inspect component multipart --allow-compat-normalization",
    );
}

#[test]
fn inspect_allows_specific_effect_when_explicit() {
    let output = run_inspect(&["--reports", "--allow-effect", "semantic_drop"]);
    common::assert_success(output, "pp inspect --allow-effect semantic_drop --reports");
}

#[test]
fn inspect_allows_specific_report_code_when_explicit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "response-variant.yaml", RESPONSE_VARIANT_SPEC);
    let output = Command::new(common::pp_bin())
        .arg("inspect")
        .arg(spec)
        .arg("--reports")
        .arg("--allow-report-code")
        .arg("spec.normalize.response_variants_pruned")
        .output()
        .expect("failed to run pp inspect");

    common::assert_success(output, "pp inspect --allow-report-code");
}

#[test]
fn inspect_allows_compat_normalization_when_explicit() {
    let output = run_inspect(&["--reports", "--allow-compat-normalization"]);
    common::assert_success(output, "pp inspect --allow-compat-normalization --reports");
}

#[test]
fn generate_writes_transform_plan_with_approval_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "lossy.yaml", LOSSY_SPEC);
    let out_dir = temp.path().join("out");
    let output = Command::new(common::pp_bin())
        .arg("generate")
        .arg(spec)
        .arg("-o")
        .arg(&out_dir)
        .arg("--base-url")
        .arg("https://example.test")
        .arg("--allow-effect")
        .arg("semantic_drop")
        .output()
        .expect("failed to run pp generate");

    common::assert_success(output, "pp generate --base-url --allow-effect");
    let plan_path = out_dir.join("pp-transform-plan.json");
    let value: Value = serde_json::from_slice(
        &std::fs::read(&plan_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", plan_path.display())),
    )
    .expect("transform plan JSON");
    assert_eq!(value["approval"]["profile"], "strict");
    assert_eq!(
        value["approval"]["allowed_effects"],
        serde_json::json!(["semantic_drop"])
    );
    assert!(value["entries"].as_array().unwrap().iter().any(|entry| {
        entry["code"] == "spec.normalize.response_variants_pruned"
            && entry["effect"] == "semantic_drop"
    }));
    assert!(value["approval"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|decision| {
            decision["code"] == "spec.normalize.response_variants_pruned"
                && decision["allowed_by"] == "effect_allowlist"
        }));
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
    assert!(
        stderr.contains("operation PATCH /items/{id} is missing operationId"),
        "stderr did not explain missing operationId:\n{stderr}"
    );
    assert!(
        stderr.contains("explicit operationId is required for codegen/MCP identity"),
        "stderr did not explain explicit operationId requirement:\n{stderr}"
    );
    assert!(
        stderr.contains("--exclude-operation \"patch /items/{id}\""),
        "stderr did not include exclude hint:\n{stderr}"
    );
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
    assert!(
        stderr.contains("no servers[0].url") && stderr.contains("--base-url"),
        "stderr did not explain missing base URL:\n{stderr}"
    );
}

#[test]
fn report_json_exposes_effect_classification() {
    let output = run_inspect(&["--reports", "--allow-compat-normalization"]);
    assert!(
        output.status.success(),
        "pp inspect --reports failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("inspect report JSON");
    let reports = value["reports"].as_array().expect("reports array");
    assert!(reports.iter().any(|report| {
        report["code"] == "spec.normalize.response_variants_pruned"
            && report["effect"] == "semantic_drop"
    }));
    assert!(reports.iter().any(|report| {
        report["code"] == "spec.normalize.content_types_pruned"
            && report["effect"] == "semantic_drop"
    }));
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
