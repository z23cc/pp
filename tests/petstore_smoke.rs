mod common;

use std::process::Command;

#[test]
#[ignore = "expensive smoke test: generates and builds a wrapper CLI; run with `cargo test -- --ignored`"]
fn petstore_generate_builds_and_lists_real_path_and_query_param_ops() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/petstore.yaml");
    let out_dir = temp.path().join("out");

    common::assert_success(
        common::run_pp_generate(&spec, &out_dir),
        "pp generate --build",
    );

    let bin = common::generated_bin(&out_dir, "swagger-petstore-open-api-3-0");
    let mut command = Command::new(&bin);
    let output = common::disable_proxy(&mut command)
        .env("SWAGGER_PETSTORE_OPEN_API_3_0_API_KEY", "dummy")
        .arg("--help")
        .output()
        .expect("failed to run generated help");
    common::assert_success(output, "generated --help");

    let help = String::from_utf8_lossy(
        &Command::new(&bin)
            .env("SWAGGER_PETSTORE_OPEN_API_3_0_API_KEY", "dummy")
            .arg("--help")
            .output()
            .expect("failed to rerun generated help")
            .stdout,
    )
    .into_owned();
    assert!(
        help.contains("get-pet-by-id") || help.contains("get_pet_by_id"),
        "help did not list get-pet-by-id/get_pet_by_id:\n{help}"
    );
    assert!(
        help.contains("find-pets-by-status") || help.contains("find_pets_by_status"),
        "help did not list find-pets-by-status/find_pets_by_status:\n{help}"
    );
}
