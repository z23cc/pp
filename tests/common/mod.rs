use std::path::Path;
use std::process::{Command, Output};

pub fn pp_bin() -> &'static str {
    env!("CARGO_BIN_EXE_pp")
}

#[allow(dead_code)]
pub fn run_pp_generate(spec: &Path, out_dir: &Path) -> Output {
    Command::new(pp_bin())
        .arg("generate")
        .arg(spec)
        .arg("-o")
        .arg(out_dir)
        .arg("--allow-compat-normalization")
        .arg("--build")
        .output()
        .expect("failed to run pp generate")
}

pub fn assert_success(output: Output, label: &str) {
    assert!(
        output.status.success(),
        "{label} failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[allow(dead_code)]
pub fn generated_bin(out_dir: &Path, bin_name: &str) -> std::path::PathBuf {
    out_dir.join("target").join("release").join(bin_name)
}

#[allow(dead_code)]
pub fn disable_proxy(command: &mut Command) -> &mut Command {
    command
        .env("NO_PROXY", "*")
        .env("no_proxy", "*")
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
}

#[allow(dead_code)]
pub fn write_spec(dir: &Path, name: &str, spec: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, spec).expect("failed to write spec");
    path
}
