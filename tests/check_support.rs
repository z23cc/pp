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
    assert!(value["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| {
            diagnostic["code"] == "direct_http.parameter_type_unsupported"
                && diagnostic["source"] == "direct_http"
        }));
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
        assert!(
            value["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| { diagnostic["code"] == expected_code }),
            "missing {expected_code} in check diagnostics: {value}"
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
