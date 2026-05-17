mod common;

#[test]
#[ignore = "expensive smoke test: verifies full Petstore fails the strict native subset; run with `cargo test -- --ignored`"]
fn petstore_generate_rejects_unsupported_native_operations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/petstore.yaml");
    let out_dir = temp.path().join("out");

    let output = common::pp_generate_command(&spec, &out_dir)
        .arg("--auth-scheme")
        .arg("api_key")
        .arg("--base-url")
        .arg("https://petstore.example.test/api/v3")
        .arg("--build")
        .output()
        .expect("failed to run pp generate");

    assert!(
        !output.status.success(),
        "full Petstore should fail when selected operations exceed the native subset"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unsupported native direct HTTP operation shape"),
        "stderr:\n{stderr}"
    );
    assert!(stderr.contains("uploadFile"), "stderr:\n{stderr}");
    assert!(!stderr.contains("spec.prepare."), "stderr:\n{stderr}");
}
