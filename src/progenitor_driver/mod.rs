//! Thin subprocess driver around `cargo-progenitor`. We shell out rather than
//! linking the progenitor library to keep version drift contained (KTD-2).

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Command;

/// Pinned cargo-progenitor version. Bump deliberately + smoke-test petstore.
pub const PINNED_VERSION: &str = "0.10";

/// Run `cargo-progenitor progenitor -i <spec> -o <out_dir> -n <name> -v 0.1.0`.
/// On failure, returns the captured stderr verbatim.
pub fn generate(spec: &Path, out_dir: &Path, name: &str) -> Result<()> {
    let status = Command::new("cargo-progenitor")
        .arg("progenitor")
        .arg("-i")
        .arg(spec)
        .arg("-o")
        .arg(out_dir)
        .arg("-n")
        .arg(name)
        .arg("-v")
        .arg("0.1.0")
        .output()
        .with_context(|| {
            "failed to spawn cargo-progenitor; install with `cargo install cargo-progenitor`"
        })?;
    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        return Err(anyhow!(
            "cargo-progenitor failed (exit {}):\n{stderr}",
            status.status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

/// Check that `cargo-progenitor` is available.
pub fn check_available() -> Result<String> {
    let out = Command::new("cargo-progenitor")
        .arg("--help")
        .output()
        .with_context(|| {
            "cargo-progenitor not found; install with `cargo install cargo-progenitor`"
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!("cargo-progenitor --help failed:\n{stderr}"));
    }
    Ok(format!(
        "cargo-progenitor available; expected {PINNED_VERSION}"
    ))
}
