mod common;

use std::process::Command;

#[test]
#[ignore = "expensive smoke test: runs cargo-progenitor and cargo build --release; run with `cargo test -- --ignored`"]
fn petstore_generate_builds_and_helps() {
    if !common::cargo_progenitor_available() {
        eprintln!(
            "skipping: cargo-progenitor is not installed; run `cargo install cargo-progenitor`"
        );
        return;
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "petstore-minimal.yaml",
        r#"
openapi: 3.0.0
info:
  title: Smoke Petstore
  version: "1.0.0"
servers:
  - url: https://petstore3.swagger.io/api/v3
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: array
                items:
                  type: object
                  properties:
                    id:
                      type: integer
                    name:
                      type: string
  /pets/{petId}:
    get:
      operationId: getPet
      parameters:
        - name: petId
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
                  id:
                    type: integer
                  name:
                    type: string
"#,
    );
    let out_dir = temp.path().join("out");

    common::assert_success(
        common::run_pp_generate(&spec, &out_dir),
        "pp generate --build",
    );

    let help = Command::new(common::generated_bin(&out_dir, "smoke-petstore"))
        .arg("--help")
        .output()
        .expect("failed to run generated --help");
    common::assert_success(help, "generated --help");
}
