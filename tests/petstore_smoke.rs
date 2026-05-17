mod common;

#[test]
#[ignore = "expensive smoke test: runs the generator until the backend rejects unsupported Petstore shapes; run with `cargo test -- --ignored`"]
fn petstore_generate_fails_naturally_on_backend_unsupported_multi_media_types() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/petstore.yaml");
    let out_dir = temp.path().join("out");

    let output = common::pp_generate_command(&spec, &out_dir)
        .arg("--auth-scheme")
        .arg("api_key")
        .arg("--build")
        .output()
        .expect("failed to run pp generate");

    assert!(
        !output.status.success(),
        "Petstore should fail naturally in the backend without spec rewrites"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("more media types than expected") || stderr.contains("not yet implemented"),
        "stderr:\n{stderr}"
    );
    assert!(!stderr.contains("spec.prepare."), "stderr:\n{stderr}");
}
