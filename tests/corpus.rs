mod common;

use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Deserialize)]
struct CorpusEntry {
    id: String,
    path: String,
    expected_check: ExpectedCheck,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    auth_scheme: Option<String>,
    #[serde(default)]
    generate_build: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ExpectedCheck {
    Pass,
    Fail,
}

#[test]
fn corpus_manifest_paths_and_expectations_are_valid() {
    let entries = corpus_entries();
    assert!(!entries.is_empty(), "corpus manifest must not be empty");
    let root = repo_root()
        .canonicalize()
        .expect("repo root should canonicalize");

    for entry in entries {
        let path = repo_root().join(&entry.path);
        let canonical_path = path
            .canonicalize()
            .unwrap_or_else(|err| panic!("fixture '{}' should canonicalize: {err}", entry.id));
        assert!(
            canonical_path.starts_with(&root),
            "corpus fixture '{}' escapes repo root: {}",
            entry.id,
            canonical_path.display()
        );
        assert_no_remote_refs(&entry, &path);
        if entry.generate_build {
            assert_eq!(
                entry.expected_check,
                ExpectedCheck::Pass,
                "only check-pass fixtures should opt into generated build coverage: {}",
                entry.id
            );
        }
    }
}

fn assert_no_remote_refs(entry: &CorpusEntry, path: &Path) {
    let body = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read corpus fixture '{}': {err}", entry.id));
    let has_remote_ref = body.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("$ref:")
            && (trimmed.contains("http://") || trimmed.contains("https://"))
    });
    assert!(
        !has_remote_ref,
        "corpus fixture '{}' must not depend on remote $ref values",
        entry.id
    );
}

#[test]
#[ignore = "local real-world corpus smoke: runs pp check --json across pinned fixtures"]
fn corpus_check_json_matches_manifest_expectations() {
    for entry in corpus_entries() {
        let output = pp_check_output(&entry);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
            panic!(
                "fixture '{}' did not emit check JSON: {err}\nstdout:\n{}\nstderr:\n{}",
                entry.id,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });

        let expected_success = entry.expected_check == ExpectedCheck::Pass;
        assert_eq!(
            output.status.success(),
            expected_success,
            "fixture '{}' process status mismatch\nstdout:\n{}\nstderr:\n{}",
            entry.id,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            value["success"].as_bool(),
            Some(expected_success),
            "fixture '{}' JSON success mismatch: {value:#}",
            entry.id
        );
        assert_eq!(value["schema_version"], "pp.check.v1");
        assert_eq!(value["support_matrix_id"], "pp.strict-openapi-support.v1");
        if expected_success {
            assert!(
                value["diagnostics"].as_array().is_some_and(Vec::is_empty),
                "fixture '{}' should have no diagnostics: {value:#}",
                entry.id
            );
        } else {
            assert!(
                value["diagnostics"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty()),
                "fixture '{}' should expose explicit diagnostics: {value:#}",
                entry.id
            );
        }
    }
}

#[test]
#[ignore = "expensive corpus coverage: generates and builds check-pass fixture CLIs"]
fn corpus_generate_builds_check_pass_fixtures() {
    for entry in corpus_entries()
        .into_iter()
        .filter(|entry| entry.generate_build)
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let out_dir = temp.path().join(&entry.id);
        let spec = repo_root().join(&entry.path);
        let mut command = common::pp_generate_command(&spec, &out_dir);
        command.arg("--build");
        if let Some(base_url) = &entry.base_url {
            command.arg("--base-url").arg(base_url);
        }
        if let Some(auth_scheme) = &entry.auth_scheme {
            command.arg("--auth-scheme").arg(auth_scheme);
        }
        let output = command.output().expect("failed to run pp generate");
        common::assert_success(
            output,
            &format!("pp generate --build corpus fixture {}", entry.id),
        );
    }
}

fn pp_check_output(entry: &CorpusEntry) -> std::process::Output {
    let mut command = Command::new(common::pp_bin());
    command
        .arg("check")
        .arg(repo_root().join(&entry.path))
        .arg("--json");
    if let Some(base_url) = &entry.base_url {
        command.arg("--base-url").arg(base_url);
    }
    if let Some(auth_scheme) = &entry.auth_scheme {
        command.arg("--auth-scheme").arg(auth_scheme);
    }
    command.output().expect("failed to run pp check")
}

fn corpus_entries() -> Vec<CorpusEntry> {
    let path = repo_root().join("tests/corpus/manifest.json");
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_json::from_str(&body)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}
