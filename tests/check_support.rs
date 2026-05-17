mod common;

use serde_json::Value;
use std::process::Command;

const MINIMAL_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Check Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /ping:
    get:
      operationId: ping
      responses:
        '200':
          description: ok
"#;

const RELATIVE_SERVER_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Relative Server Check Fixture
  version: "1.0.0"
servers:
  - url: /api/v1
paths:
  /ping:
    get:
      operationId: ping
      responses:
        '200':
          description: ok
"#;

const MISSING_OPERATION_ID_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Missing Operation ID Check Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /pets/{id}:
    get:
      responses:
        '200':
          description: ok
"#;

const DEEP_OBJECT_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Unsupported Check Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /search:
    get:
      operationId: searchThings
      parameters:
        - name: filter
          in: query
          required: true
          style: deepObject
          schema:
            type: object
            properties:
              name:
                type: string
      responses:
        '200':
          description: ok
"#;

#[test]
fn check_json_reports_success_without_writing_workspace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "minimal.yaml", MINIMAL_SPEC);

    let output = Command::new(common::pp_bin())
        .arg("check")
        .arg(&spec)
        .arg("--json")
        .output()
        .expect("failed to run pp check");

    assert!(
        output.status.success(),
        "pp check --json failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("check JSON");
    assert_eq!(value["schema_version"], "pp.check.v1");
    assert_eq!(value["support_matrix_id"], "pp.strict-openapi-support.v1");
    assert_eq!(value["success"], true);
    assert_eq!(value["facts"]["operation_count"], 1);
    assert!(value["diagnostics"].as_array().unwrap().is_empty());
    assert!(!temp.path().join("Cargo.toml").exists());
    assert!(!temp.path().join("pp-transform-plan.json").exists());
}

#[test]
fn check_json_reports_unsupported_operations_with_diagnostic_codes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "unsupported.yaml", DEEP_OBJECT_SPEC);

    let output = Command::new(common::pp_bin())
        .arg("check")
        .arg(spec)
        .arg("--json")
        .output()
        .expect("failed to run pp check");

    assert!(
        !output.status.success(),
        "unsupported check unexpectedly succeeded"
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("check JSON");
    assert_eq!(value["success"], false);
    assert_eq!(
        value["unsupported_operations"][0]["operation_id"],
        "searchThings"
    );
    assert_eq!(
        value["unsupported_operations"][0]["diagnostic_code"],
        "direct_http.parameter_type_unsupported"
    );
    let diagnostic = value["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| {
            diagnostic["code"] == "direct_http.parameter_type_unsupported"
                && diagnostic["source"] == "direct_http"
        })
        .expect("direct_http diagnostic");
    assert_eq!(diagnostic["title"], "Parameter type is unsupported");
    assert!(diagnostic["remediation"]
        .as_str()
        .unwrap()
        .contains("supported primitive"));
    assert!(diagnostic["strict_behavior"]
        .as_str()
        .unwrap()
        .contains("does not generate fallback"));
    assert!(diagnostic["support_features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|feature| { feature.as_str() == Some("parameters.path_query_primitives") }));
    assert!(value["unsupported_operations"][0]["support_features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|feature| feature.as_str() == Some("parameters.path_query_primitives")));
}

#[test]
fn check_json_public_diagnostic_codes_resolve_in_support_inventory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let invalid_spec = common::write_spec(temp.path(), "invalid.yaml", "not: [valid");
    let relative_spec = common::write_spec(temp.path(), "relative.yaml", RELATIVE_SERVER_SPEC);
    let missing_operation_id_spec = common::write_spec(
        temp.path(),
        "missing-operation-id.yaml",
        MISSING_OPERATION_ID_SPEC,
    );
    let unsupported_spec = common::write_spec(temp.path(), "unsupported.yaml", DEEP_OBJECT_SPEC);

    let cases = [
        (invalid_spec, "spec.load_error"),
        (relative_spec, "runtime.base_url"),
        (missing_operation_id_spec, "model.generation_error"),
        (unsupported_spec, "direct_http.parameter_type_unsupported"),
    ];

    for (spec, expected_code) in cases {
        let output = Command::new(common::pp_bin())
            .arg("check")
            .arg(spec)
            .arg("--json")
            .output()
            .expect("failed to run pp check");
        assert!(
            !output.status.success(),
            "{expected_code} check unexpectedly succeeded"
        );
        let value: Value = serde_json::from_slice(&output.stdout).expect("check JSON");
        let diagnostic = value["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|diagnostic| diagnostic["code"] == expected_code)
            .unwrap_or_else(|| panic!("missing {expected_code} in check diagnostics: {value}"));
        assert!(
            diagnostic["title"]
                .as_str()
                .is_some_and(|title| !title.is_empty()),
            "missing title for {expected_code}: {diagnostic}"
        );
        assert!(
            diagnostic["remediation"]
                .as_str()
                .is_some_and(|remediation| !remediation.is_empty()),
            "missing remediation for {expected_code}: {diagnostic}"
        );
        assert!(
            diagnostic["severity_hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("error:")),
            "missing severity hint for {expected_code}: {diagnostic}"
        );
        assert!(
            diagnostic["strict_behavior"]
                .as_str()
                .is_some_and(|behavior| !behavior.is_empty()),
            "missing strict behavior for {expected_code}: {diagnostic}"
        );
        assert!(
            diagnostic["support_features"]
                .as_array()
                .is_some_and(|features| !features.is_empty()),
            "missing support features for {expected_code}: {diagnostic}"
        );

        let diagnostic_output = Command::new(common::pp_bin())
            .arg("support")
            .arg("--diagnostic")
            .arg(expected_code)
            .arg("--json")
            .output()
            .expect("failed to run pp support --diagnostic");
        common::assert_success(diagnostic_output, "pp support --diagnostic --json");
    }
}

#[test]
fn explain_human_and_json_use_support_inventory() {
    let human_output = Command::new(common::pp_bin())
        .arg("explain")
        .arg("direct_http.request_body_json_missing")
        .output()
        .expect("failed to run pp explain");
    assert!(
        human_output.status.success(),
        "pp explain failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&human_output.stdout),
        String::from_utf8_lossy(&human_output.stderr)
    );
    let human_stdout = String::from_utf8_lossy(&human_output.stdout);
    assert!(
        human_stdout.contains("Severity:"),
        "stdout:\n{human_stdout}"
    );
    assert!(
        human_stdout.contains("Strict behavior:"),
        "stdout:\n{human_stdout}"
    );
    assert!(
        human_stdout.contains("does not generate fallback"),
        "stdout:\n{human_stdout}"
    );

    let json_output = Command::new(common::pp_bin())
        .arg("explain")
        .arg("direct_http.request_body_json_missing")
        .arg("--json")
        .output()
        .expect("failed to run pp explain --json");
    assert!(
        json_output.status.success(),
        "pp explain --json failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&json_output.stdout),
        String::from_utf8_lossy(&json_output.stderr)
    );
    let value: Value = serde_json::from_slice(&json_output.stdout).expect("explain JSON");
    assert_eq!(value["matrix_id"], "pp.strict-openapi-support.v1");
    assert_eq!(
        value["diagnostic_code"],
        "direct_http.request_body_json_missing"
    );
    assert!(value["meaning"].as_str().unwrap().contains("request body"));
    assert!(value["severity_hint"].as_str().unwrap().contains("error:"));
    assert!(value["strict_behavior"]
        .as_str()
        .unwrap()
        .contains("does not generate fallback"));
    assert!(value["features"].as_array().unwrap().iter().any(|feature| {
        feature["id"] == "request_bodies.json" && feature["status"] == "supported"
    }));
}

#[test]
fn explain_spec_load_error_covers_all_pre_model_failures() {
    let json_output = Command::new(common::pp_bin())
        .arg("explain")
        .arg("spec.load_error")
        .arg("--json")
        .output()
        .expect("failed to run pp explain spec.load_error --json");
    assert!(
        json_output.status.success(),
        "pp explain spec.load_error --json failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&json_output.stdout),
        String::from_utf8_lossy(&json_output.stderr)
    );
    let value: Value = serde_json::from_slice(&json_output.stdout).expect("explain JSON");
    assert_eq!(value["diagnostic_code"], "spec.load_error");
    let text = format!(
        "{} {}",
        value["meaning"].as_str().unwrap(),
        value["remediation"].as_str().unwrap()
    );
    for expected in ["reading", "parsing", "slice", "auth scheme"] {
        assert!(text.contains(expected), "missing {expected} in: {text}");
    }
}

#[test]
fn explain_unknown_diagnostic_fails_clearly() {
    let output = Command::new(common::pp_bin())
        .arg("explain")
        .arg("not.real")
        .output()
        .expect("failed to run pp explain");

    assert!(
        !output.status.success(),
        "unknown diagnostic unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown diagnostic code 'not.real'"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn check_human_success_prints_spec_summary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "minimal.yaml", MINIMAL_SPEC);

    let output = Command::new(common::pp_bin())
        .arg("check")
        .arg(spec)
        .output()
        .expect("failed to run pp check");
    assert!(
        output.status.success(),
        "pp check failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("pp check: ok"), "stdout:\n{stdout}");
    assert!(stdout.contains("Spec:"), "stdout:\n{stdout}");
    assert!(stdout.contains("title: Check Fixture"), "stdout:\n{stdout}");
    assert!(stdout.contains("operations: 1"), "stdout:\n{stdout}");
    assert!(stdout.contains("auth: none"), "stdout:\n{stdout}");
}

#[test]
fn check_human_failure_prints_diagnostics_and_explain_hint() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "unsupported.yaml", DEEP_OBJECT_SPEC);

    let output = Command::new(common::pp_bin())
        .arg("check")
        .arg(spec)
        .output()
        .expect("failed to run pp check");

    assert!(
        !output.status.success(),
        "unsupported check unexpectedly succeeded"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("pp check: failed"), "stdout:\n{stdout}");
    assert!(stdout.contains("Spec:"), "stdout:\n{stdout}");
    assert!(stdout.contains("Diagnostics:"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("direct_http.parameter_type_unsupported"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("title: Parameter type is unsupported"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("related support features: parameters.path_query_primitives"),
        "stdout:\n{stdout}"
    );
    assert!(stdout.contains("strict behavior:"), "stdout:\n{stdout}");
    assert!(stdout.contains("remediation:"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("Unsupported operations:"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("direct_http.parameter_type_unsupported (1 operation)"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("Run: pp explain direct_http.parameter_type_unsupported"),
        "stdout:\n{stdout}"
    );
}

#[test]
fn support_json_and_queries_are_backed_by_matrix_inventory() {
    let output = Command::new(common::pp_bin())
        .arg("support")
        .arg("--json")
        .output()
        .expect("failed to run pp support");
    common::assert_success(output, "pp support --json");

    let feature_output = Command::new(common::pp_bin())
        .arg("support")
        .arg("--feature")
        .arg("openapi.3_0.strict_subset")
        .arg("--json")
        .output()
        .expect("failed to run pp support --feature");
    common::assert_success(feature_output, "pp support --feature --json");

    let diagnostic_output = Command::new(common::pp_bin())
        .arg("support")
        .arg("--diagnostic")
        .arg("direct_http.request_body_json_missing")
        .arg("--json")
        .output()
        .expect("failed to run pp support --diagnostic");
    let value: Value = serde_json::from_slice(&diagnostic_output.stdout).expect("support JSON");
    assert_eq!(
        value["diagnostic_code"],
        "direct_http.request_body_json_missing"
    );
    assert!(value["features"].as_array().unwrap().iter().any(|feature| {
        feature["id"] == "request_bodies.json" && feature["status"] == "supported"
    }));

    let schema_diagnostic_output = Command::new(common::pp_bin())
        .arg("support")
        .arg("--diagnostic")
        .arg("schema.keyword_unsupported")
        .arg("--json")
        .output()
        .expect("failed to run pp support --diagnostic schema");
    assert!(
        schema_diagnostic_output.status.success(),
        "pp support --diagnostic schema --json failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&schema_diagnostic_output.stdout),
        String::from_utf8_lossy(&schema_diagnostic_output.stderr)
    );
    let schema_value: Value =
        serde_json::from_slice(&schema_diagnostic_output.stdout).expect("support schema JSON");
    assert_eq!(
        schema_value["diagnostic_code"],
        "schema.keyword_unsupported"
    );
    assert!(schema_value["features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|feature| {
            feature["id"] == "json_schema.broad_2020_12" && feature["status"] == "unsupported"
        }));
}

#[test]
fn support_unknown_feature_fails_clearly() {
    let output = Command::new(common::pp_bin())
        .arg("support")
        .arg("--feature")
        .arg("not.real")
        .arg("--json")
        .output()
        .expect("failed to run pp support --feature");

    assert!(
        !output.status.success(),
        "unknown feature unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown support feature 'not.real'"),
        "stderr:\n{stderr}"
    );
}
