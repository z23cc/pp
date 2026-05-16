mod common;

use serde_json::Value;
use std::process::{Command, Output};

const SLICING_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Slicing Fixture
  version: "1.0.0"
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
    assert_eq!(rows[1]["id"], "getPet");
    assert_eq!(rows[1]["path"], "/pets/{id}");
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
