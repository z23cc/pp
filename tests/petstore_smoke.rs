mod common;

use std::process::Command;

#[test]
#[ignore = "expensive smoke test: generates and builds a wrapper CLI; run with `cargo test -- --ignored`"]
fn petstore_generate_builds_and_handles_path_and_query_params() {
    let mut server = mockito::Server::new();
    let list_mock = server
        .mock("GET", "/pets")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"[{"id":1,"name":"spot"}]"#)
        .create();
    let path_mock = server
        .mock("GET", "/pet/123")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"id":123,"name":"spot"}"#)
        .create();
    let query_mock = server
        .mock("GET", "/pet/findByStatus")
        .match_query(mockito::Matcher::UrlEncoded(
            "status".to_string(),
            "available".to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"[{"id":2,"name":"fluffy"}]"#)
        .create();

    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "petstore-minimal.yaml",
        &format!(
            r#"
openapi: 3.0.0
info:
  title: Smoke Petstore
  version: "1.0.0"
servers:
  - url: {}
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
                  $ref: '#/components/schemas/Pet'
  /pet/{{petId}}:
    get:
      operationId: getPetById
      parameters:
        - name: petId
          in: path
          required: true
          schema:
            type: integer
            format: int64
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
  /pet/findByStatus:
    get:
      operationId: findPetsByStatus
      parameters:
        - name: status
          in: query
          required: true
          schema:
            type: string
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: '#/components/schemas/Pet'
components:
  schemas:
    Pet:
      type: object
      properties:
        id:
          type: integer
          format: int64
        name:
          type: string
"#,
            server.url()
        ),
    );
    let out_dir = temp.path().join("out");

    common::assert_success(
        common::run_pp_generate(&spec, &out_dir),
        "pp generate --build",
    );

    let bin = common::generated_bin(&out_dir, "smoke-petstore");
    for (args, label) in [
        (vec!["list_pets"], "generated list_pets"),
        (
            vec!["get_pet_by_id", "--pet-id", "123"],
            "generated get_pet_by_id",
        ),
        (
            vec!["find_pets_by_status", "--status", "available"],
            "generated find_pets_by_status",
        ),
    ] {
        let mut command = Command::new(&bin);
        let output = common::disable_proxy(&mut command)
            .args(args)
            .output()
            .expect("failed to run generated command");
        common::assert_success(output, label);
    }

    list_mock.assert();
    path_mock.assert();
    query_mock.assert();
}
