mod common;

use std::process::Command;

#[test]
#[ignore = "expensive smoke test: runs cargo-progenitor and cargo build --release; run with `cargo test -- --ignored`"]
fn api_key_header_is_sent() {
    if !common::cargo_progenitor_available() {
        eprintln!(
            "skipping: cargo-progenitor is not installed; run `cargo install cargo-progenitor`"
        );
        return;
    }

    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/ping")
        .match_request(|request| {
            request
                .header("x-api-key")
                .iter()
                .any(|value| value.to_str().ok() == Some("foo"))
        })
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true}"#)
        .create();

    let temp = tempfile::tempdir().expect("tempdir");
    let spec = common::write_spec(
        temp.path(),
        "apikey.yaml",
        &format!(
            r#"
openapi: 3.0.0
info:
  title: My API
  version: "1.0.0"
servers:
  - url: {}
security:
  - apiKeyAuth: []
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
    apiKeyAuth:
      type: apiKey
      in: header
      name: X-API-Key
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
        .arg("get-ping")
        .env("MY_API_API_KEY", "foo")
        .output()
        .expect("failed to run generated command");
    common::assert_success(output, "generated get-ping");
    mock.assert();
}
