use std::process::{Command, Output};

fn pp_bin() -> &'static str {
    env!("CARGO_BIN_EXE_pp")
}

fn assert_success(output: Output, label: &str) {
    assert!(
        output.status.success(),
        "{label} failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_workspace(dir: &std::path::Path, main_rs: &str) {
    std::fs::write(
        dir.join("Cargo.toml"),
        r#"[package]
name = "validate-fixture"
version = "0.0.0"
edition = "2021"
"#,
    )
    .expect("write Cargo.toml");
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(dir.join("src/main.rs"), main_rs).expect("write main.rs");
}

#[test]
fn validate_builds_generated_workspace() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_workspace(temp.path(), "fn main() {}\n");

    let output = Command::new(pp_bin())
        .arg("validate")
        .arg(temp.path())
        .output()
        .expect("failed to run pp validate");

    assert_success(output, "pp validate");
    assert!(temp.path().join("target/release/validate-fixture").exists());
}

#[test]
fn validate_reports_build_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_workspace(temp.path(), "fn main() { missing_symbol(); }\n");

    let output = Command::new(pp_bin())
        .arg("validate")
        .arg(temp.path())
        .output()
        .expect("failed to run pp validate");

    assert!(
        !output.status.success(),
        "pp validate unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("pp: running `cargo build --release`"),
        "stderr did not include build start message:\n{stderr}"
    );
    assert!(
        stderr.contains("cargo build --release failed"),
        "stderr did not include build failure message:\n{stderr}"
    );
}
