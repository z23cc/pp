mod common;

use serde_json::Value;
use std::process::{Command, Output};

const SLICING_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Slicing Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /pets:
    get:
      operationId: listPets
      tags: [pets]
      responses:
        '200':
          description: ok
  /pets/{id}:
    get:
      operationId: getPet
      tags: [pets]
      responses:
        '200':
          description: ok
  /store/orders:
    post:
      operationId: createOrder
      tags: [store]
      responses:
        '201':
          description: created
"#;

// Fast PR checks: these exercise inspect-only slicing behavior without generating
// or building a workspace. Generated-workspace smoke tests remain #[ignore]
// because they compile fixture CLIs and are intended for manual/deep runs.

#[test]
fn inspect_include_operation_slices_to_one_operation() {
    let output = run_inspect(&["--include-operation", "getPet"]);
    common::assert_success(output, "pp inspect --include-operation");
}

#[test]
fn inspect_slicing_flags_update_operation_count() {
    assert_operation_count(&["--include-operation", "getPet"], 1);
    assert_operation_count(&["--include-tag", "pets"], 2);
    assert_operation_count(&["--include-path-prefix", "/store"], 1);
    assert_operation_count(&["--exclude-operation", "listPets"], 2);
    assert_operation_count(
        &["--include-tag", "pets", "--exclude-operation", "getPet"],
        1,
    );
}

#[test]
fn inspect_list_operations_prints_filtered_jsonl_rows() {
    let output = run_inspect(&["--list-operations", "--include-path-prefix", "/pets"]);
    assert!(
        output.status.success(),
        "inspect list failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let rows: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("operation listing row is JSON"))
        .collect();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], "listPets");
    assert_eq!(rows[0]["method"], "get");
    assert_eq!(rows[0]["path"], "/pets");
    assert_eq!(rows[0]["tags"], serde_json::json!(["pets"]));
    assert_eq!(rows[0]["operation_id"], "listPets");
    assert_eq!(rows[0]["derived_id"], "get /pets");
    assert_eq!(rows[0]["generatable"], true);
    assert_eq!(rows[1]["id"], "getPet");
    assert_eq!(rows[1]["path"], "/pets/{id}");
}

#[test]
fn inspect_list_operations_marks_missing_operation_id_as_not_generatable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "missing-operation-id.yaml",
        r#"
openapi: 3.0.0
info:
  title: Missing Operation ID Fixture
  version: "1.0.0"
paths:
  /items/{id}:
    patch:
      tags: [items]
      responses:
        '200':
          description: ok
"#,
    );
    let output = Command::new(common::pp_bin())
        .arg("inspect")
        .arg(spec)
        .arg("--list-operations")
        .output()
        .expect("failed to run pp inspect --list-operations");
    assert!(
        output.status.success(),
        "pp inspect --list-operations missing operationId failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let row: Value = serde_json::from_slice(&output.stdout).expect("operation listing row is JSON");
    assert_eq!(row["id"], "patch /items/{id}");
    assert_eq!(row["operation_id"], Value::Null);
    assert_eq!(row["derived_id"], "patch /items/{id}");
    assert_eq!(row["generatable"], false);
}

#[test]
fn inspect_reports_outputs_facts_and_structured_reports() {
    let output = run_inspect(&["--reports", "--include-tag", "pets"]);
    assert!(
        output.status.success(),
        "inspect --reports failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("inspect reports stdout is JSON");
    assert_eq!(payload["facts"]["operation_count"].as_u64(), Some(2));
    assert_eq!(payload["auth_plan"]["selected"]["kind"], "none");
    assert_eq!(payload["auth_plan"]["decision"]["kind"], "none");
    assert_eq!(
        payload["auth_plan"]["candidates"]
            .as_array()
            .expect("auth candidates is an array")
            .len(),
        0
    );

    let reports = payload["reports"].as_array().expect("reports is an array");
    assert!(
        reports.iter().any(|report| report["stage"] == "slicing"
            && report["severity"] == "warning"
            && report["code"] == "spec.slice.operations_filtered"),
        "missing structured slicing report: {reports:#?}"
    );
    assert!(
        reports
            .iter()
            .any(|report| report["code"] == "spec.slice.components_pruned"
                && report["subject"]["kind"] == "component"
                && report["subject"]["value"] == "components"),
        "missing structured component-pruning subject: {reports:#?}"
    );
}

#[test]
fn inspect_empty_slice_fails_with_discovery_hint() {
    let output = run_inspect(&["--include-tag", "missing"]);
    assert!(
        !output.status.success(),
        "empty slice unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no operations matched slice filters; use `pp inspect --list-operations` to discover operation IDs/tags"),
        "stderr did not contain discovery hint:\n{stderr}"
    );
}

fn assert_operation_count(args: &[&str], expected: u64) {
    let output = run_inspect(args);
    assert!(
        output.status.success(),
        "inspect failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let facts: Value = serde_json::from_slice(&output.stdout).expect("inspect stdout is JSON");
    assert_eq!(facts["operation_count"].as_u64(), Some(expected));
}

#[test]
#[ignore = "expensive smoke test: generates and builds a sliced wrapper CLI; run with `cargo test --test slicing -- --ignored`"]
fn petstore_store_slice_generates_and_builds() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/petstore.yaml");
    let out_dir = temp.path().join("out");

    let output = Command::new(common::pp_bin())
        .arg("generate")
        .arg(&spec)
        .arg("-o")
        .arg(&out_dir)
        .arg("--include-tag")
        .arg("store")
        .arg("--allow-report-code")
        .arg("spec.normalize.response_variants_pruned")
        .arg("--allow-report-code")
        .arg("spec.normalize.content_types_pruned")
        .arg("--build")
        .output()
        .expect("failed to run sliced pp generate");
    common::assert_success(output, "pp generate --include-tag store --build");

    let bin = common::generated_bin(&out_dir, "swagger-petstore");
    let mut command = Command::new(&bin);
    let output = common::disable_proxy(&mut command)
        .env("SWAGGER_PETSTORE_API_KEY", "dummy")
        .arg("--help")
        .output()
        .expect("failed to run sliced generated help");
    let help = String::from_utf8_lossy(&output.stdout).into_owned();
    common::assert_success(output, "sliced generated --help");
    assert!(
        help.contains("get-inventory") || help.contains("get_inventory"),
        "store slice help did not list get-inventory/get_inventory:\n{help}"
    );
    assert!(
        !help.contains("get-pet-by-id") && !help.contains("get_pet_by_id"),
        "store slice help unexpectedly listed pet operations:\n{help}"
    );
}

fn run_inspect(args: &[&str]) -> Output {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "slicing.yaml", SLICING_SPEC);
    Command::new(common::pp_bin())
        .arg("inspect")
        .arg(spec)
        .args(args)
        .output()
        .expect("failed to run pp inspect")
}
