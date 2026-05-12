mod common;

use std::process::Command;

#[test]
#[ignore = "expensive smoke test: generates and builds a wrapper CLI; run with `cargo test -- --ignored`"]
fn bearer_token_is_sent() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/ping")
        .match_request(|request| {
            request
                .header("authorization")
                .iter()
                .any(|value| value.to_str().ok() == Some("Bearer foo"))
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true}"#)
        .create();

    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "bearer.yaml",
        &format!(
            r#"
openapi: 3.0.0
info:
  title: My API
  version: "1.0.0"
servers:
  - url: {}
security:
  - bearerAuth: []
paths:
  /ping:
    get:
      operationId: getPing
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  ok:
                    type: boolean
components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
"#,
            server.url()
        ),
    );
    let out_dir = temp.path().join("out");

    common::assert_success(
        common::run_pp_generate(&spec, &out_dir),
        "pp generate --build",
    );

    let mut command = Command::new(common::generated_bin(&out_dir, "my-api"));
    let output = common::disable_proxy(&mut command)
        .arg("get_ping")
        .env("MY_API_TOKEN", "foo")
        .output()
        .expect("failed to run generated command");
    common::assert_success(output, "generated get-ping");
    mock.assert();
}
