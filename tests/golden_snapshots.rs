mod common;

use std::path::Path;

const NATIVE_CORE_SPEC: &str = r#"
openapi: 3.0.3
info:
  title: Native Core API
  version: "1.0.0"
servers:
  - url: https://upstream.example/api
security:
  - bearerAuth: []
paths:
  /items/{itemId}:
    get:
      operationId: getItem
      summary: Fetch one item
      parameters:
        - name: itemId
          in: path
          required: true
          schema:
            type: string
        - name: include_details
          in: query
          required: false
          schema:
            type: boolean
        - name: tag
          in: query
          required: false
          schema:
            type: array
            items:
              type: string
      responses:
        '200':
          description: item response
          content:
            application/json:
              schema:
                type: object
                required: [id, name]
                properties:
                  id:
                    type: string
                  name:
                    type: string
                  active:
                    type: boolean
  /items:
    post:
      operationId: createItem
      summary: Create one item
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              required: [name]
              properties:
                name:
                  type: string
                count:
                  type: integer
                enabled:
                  type: boolean
      responses:
        '201':
          description: created item
          content:
            application/json:
              schema:
                type: object
                properties:
                  id:
                    type: string
components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
"#;

const SNAPSHOT_FILES: &[&str] = &[
    "Cargo.toml",
    "src/main.rs",
    "src/auth.rs",
    "src/cli_builder.rs",
    "src/context.rs",
    "src/invoke.rs",
    "src/direct_http.rs",
    "src/mcp.rs",
    "src/print.rs",
    "src/runtime.rs",
    "pp-transform-plan.json",
];

#[test]
fn generated_native_core_output_matches_committed_golden() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(temp.path(), "native-core.yaml", NATIVE_CORE_SPEC);
    let out_dir = temp.path().join("out");

    let output = common::pp_generate_command(&spec, &out_dir)
        .arg("--base-url")
        .arg("https://example.test/api")
        .output()
        .expect("failed to run pp generate");
    common::assert_success(output, "pp generate native core snapshot fixture");

    let golden_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/native_core");
    for relative in SNAPSHOT_FILES {
        assert_snapshot_file(&golden_dir, &out_dir, relative);
    }
}

fn assert_snapshot_file(golden_dir: &Path, generated_dir: &Path, relative: &str) {
    let golden_path = golden_dir.join(relative);
    let generated_path = generated_dir.join(relative);
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|err| panic!("failed to read golden {}: {err}", display(&golden_path)));
    let generated = std::fs::read_to_string(&generated_path).unwrap_or_else(|err| {
        panic!(
            "failed to read generated {}: {err}",
            display(&generated_path)
        )
    });

    assert_eq!(
        normalize_trailing_ws(&golden),
        normalize_trailing_ws(&generated),
        "generated file drifted: {relative}"
    );
}

fn normalize_trailing_ws(input: &str) -> String {
    let input = input.replace("\r\n", "\n");
    let mut normalized = String::new();
    for segment in input.split_inclusive('\n') {
        if let Some(line) = segment.strip_suffix('\n') {
            normalized.push_str(line.trim_end());
            normalized.push('\n');
        } else {
            normalized.push_str(segment.trim_end());
        }
    }
    normalized
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
