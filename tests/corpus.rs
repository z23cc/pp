mod common;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Deserialize)]
struct CorpusEntry {
    id: String,
    path: String,
    source: String,
    fixture_kind: FixtureKind,
    expected_check: ExpectedCheck,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    auth_scheme: Option<String>,
    #[serde(default)]
    generate_build: bool,
    #[serde(default)]
    coverage_tags: Vec<String>,
    #[serde(default)]
    expected_diagnostics: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ExpectedCheck {
    Pass,
    Fail,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FixtureKind {
    UpstreamOpenapiTrim,
    UpstreamOpenapiFull,
    CuratedPublicApiShape,
}

impl FixtureKind {
    fn as_str(&self) -> &'static str {
        match self {
            FixtureKind::UpstreamOpenapiTrim => "upstream_openapi_trim",
            FixtureKind::UpstreamOpenapiFull => "upstream_openapi_full",
            FixtureKind::CuratedPublicApiShape => "curated_public_api_shape",
        }
    }
}

#[derive(Debug, Serialize)]
struct CorpusCoverageReport {
    schema_version: &'static str,
    totals: CorpusCoverageTotals,
    diagnostic_code_frequency: BTreeMap<String, usize>,
    coverage_tag_frequency: BTreeMap<String, usize>,
    entries: Vec<CorpusEntryReport>,
}

#[derive(Debug, Serialize)]
struct CorpusCoverageTotals {
    total: usize,
    actual_pass: usize,
    actual_fail: usize,
    expected_pass: usize,
    expected_fail: usize,
    unexpected_status_mismatches: usize,
    generate_build: usize,
}

#[derive(Debug, Serialize)]
struct CorpusEntryReport {
    id: String,
    path: String,
    source: String,
    fixture_kind: FixtureKind,
    expected_check: ExpectedCheck,
    actual_success: bool,
    coverage_tags: Vec<String>,
    expected_diagnostics: Vec<String>,
    actual_diagnostics: Vec<String>,
    generate_build: bool,
    status_matches_expectation: bool,
}

#[test]
fn corpus_manifest_paths_and_expectations_are_valid() {
    let entries = corpus_entries();
    assert!(
        entries.len() >= 20,
        "corpus manifest must include at least 20 local curated public API-shape fixtures"
    );
    let root = repo_root()
        .canonicalize()
        .expect("repo root should canonicalize");
    let mut ids = BTreeSet::new();
    let mut resolved_diagnostics = BTreeSet::new();
    let mut fixture_kinds = BTreeSet::new();

    for entry in entries {
        assert!(
            ids.insert(entry.id.clone()),
            "duplicate corpus id {}",
            entry.id
        );
        assert!(!entry.id.trim().is_empty(), "corpus id must not be empty");
        assert!(
            !entry.source.trim().is_empty(),
            "corpus fixture '{}' must document source/provenance",
            entry.id
        );
        fixture_kinds.insert(entry.fixture_kind.as_str());
        assert!(
            !entry.coverage_tags.is_empty(),
            "corpus fixture '{}' must declare coverage_tags",
            entry.id
        );
        assert!(
            entry.coverage_tags.iter().all(|tag| !tag.trim().is_empty()),
            "corpus fixture '{}' has an empty coverage tag",
            entry.id
        );
        if entry.expected_check == ExpectedCheck::Pass {
            assert!(
                entry.expected_diagnostics.is_empty(),
                "check-pass fixture '{}' should not declare expected diagnostics",
                entry.id
            );
        } else {
            assert!(
                !entry.expected_diagnostics.is_empty(),
                "check-fail fixture '{}' must declare expected diagnostics",
                entry.id
            );
        }

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
        for code in &entry.expected_diagnostics {
            if resolved_diagnostics.insert(code.clone()) {
                assert_diagnostic_resolves(code);
            }
        }
    }
    assert!(
        fixture_kinds.contains("upstream_openapi_trim"),
        "corpus manifest should distinguish upstream OpenAPI-derived trims"
    );
    assert!(
        fixture_kinds.contains("upstream_openapi_full"),
        "corpus manifest should distinguish full upstream OpenAPI fixtures"
    );
    assert!(
        fixture_kinds.contains("curated_public_api_shape"),
        "corpus manifest should distinguish hand-curated public API-shape fixtures"
    );
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

fn assert_diagnostic_resolves(code: &str) {
    let output = Command::new(common::pp_bin())
        .arg("support")
        .arg("--diagnostic")
        .arg(code)
        .arg("--json")
        .output()
        .expect("failed to run pp support --diagnostic");
    assert!(
        output.status.success(),
        "expected diagnostic '{code}' should resolve in support inventory\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|err| panic!("diagnostic '{code}' did not emit JSON: {err}"));
    assert_eq!(value["diagnostic_code"], code);
    assert_eq!(value["matrix_id"], "pp.strict-openapi-support.v1");
}

#[test]
#[ignore = "local corpus smoke: runs pp check --json across curated public API-shape fixtures"]
fn corpus_check_json_matches_manifest_expectations() {
    let mut reports = Vec::new();

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

        let actual_diagnostics = diagnostic_codes(&value);
        if expected_success {
            assert!(
                actual_diagnostics.is_empty(),
                "fixture '{}' should have no diagnostics: {value:#}",
                entry.id
            );
        } else {
            assert!(
                !actual_diagnostics.is_empty(),
                "fixture '{}' should expose explicit diagnostics: {value:#}",
                entry.id
            );
            for expected_code in &entry.expected_diagnostics {
                assert!(
                    actual_diagnostics.contains(expected_code),
                    "fixture '{}' missing expected diagnostic {expected_code}; actual: {actual_diagnostics:?}",
                    entry.id
                );
            }
        }

        reports.push(CorpusEntryReport {
            id: entry.id,
            path: entry.path,
            source: entry.source,
            fixture_kind: entry.fixture_kind,
            expected_check: entry.expected_check,
            actual_success: value["success"].as_bool().unwrap_or(false),
            coverage_tags: entry.coverage_tags,
            expected_diagnostics: entry.expected_diagnostics,
            actual_diagnostics,
            generate_build: entry.generate_build,
            status_matches_expectation: output.status.success() == expected_success,
        });
    }

    let report = build_coverage_report(reports);
    write_coverage_reports(&report);
    assert_eq!(
        report.totals.unexpected_status_mismatches, 0,
        "corpus report should not contain status mismatches"
    );
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

fn diagnostic_codes(value: &Value) -> Vec<String> {
    let mut codes: Vec<_> = value["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|diagnostic| diagnostic["code"].as_str())
        .map(str::to_owned)
        .collect();
    codes.sort();
    codes.dedup();
    codes
}

fn build_coverage_report(entries: Vec<CorpusEntryReport>) -> CorpusCoverageReport {
    let mut diagnostic_code_frequency = BTreeMap::new();
    let mut coverage_tag_frequency = BTreeMap::new();
    let mut actual_pass = 0;
    let mut actual_fail = 0;
    let mut expected_pass = 0;
    let mut expected_fail = 0;
    let mut unexpected_status_mismatches = 0;
    let mut generate_build = 0;

    for entry in &entries {
        if entry.actual_success {
            actual_pass += 1;
        } else {
            actual_fail += 1;
        }
        match entry.expected_check {
            ExpectedCheck::Pass => expected_pass += 1,
            ExpectedCheck::Fail => expected_fail += 1,
        }
        if !entry.status_matches_expectation {
            unexpected_status_mismatches += 1;
        }
        if entry.generate_build {
            generate_build += 1;
        }
        for code in &entry.actual_diagnostics {
            *diagnostic_code_frequency.entry(code.clone()).or_insert(0) += 1;
        }
        for tag in &entry.coverage_tags {
            *coverage_tag_frequency.entry(tag.clone()).or_insert(0) += 1;
        }
    }

    CorpusCoverageReport {
        schema_version: "pp.corpus_coverage.v1",
        totals: CorpusCoverageTotals {
            total: entries.len(),
            actual_pass,
            actual_fail,
            expected_pass,
            expected_fail,
            unexpected_status_mismatches,
            generate_build,
        },
        diagnostic_code_frequency,
        coverage_tag_frequency,
        entries,
    }
}

fn write_coverage_reports(report: &CorpusCoverageReport) {
    let target = repo_root().join("target");
    std::fs::create_dir_all(&target)
        .unwrap_or_else(|err| panic!("failed to create {}: {err}", target.display()));
    let json_path = target.join("pp-corpus-coverage.json");
    let markdown_path = target.join("pp-corpus-coverage.md");
    let json = serde_json::to_string_pretty(report).expect("serialize corpus coverage report");
    std::fs::write(&json_path, format!("{json}\n"))
        .unwrap_or_else(|err| panic!("failed to write {}: {err}", json_path.display()));
    std::fs::write(&markdown_path, coverage_markdown(report))
        .unwrap_or_else(|err| panic!("failed to write {}: {err}", markdown_path.display()));
    eprintln!(
        "wrote corpus coverage reports: {}, {}",
        json_path.display(),
        markdown_path.display()
    );
}

fn coverage_markdown(report: &CorpusCoverageReport) -> String {
    let mut out = String::new();
    out.push_str("# pp corpus coverage\n\n");
    out.push_str(&format!("Schema version: `{}`\n\n", report.schema_version));
    out.push_str("## Totals\n\n");
    out.push_str(&format!("- Total fixtures: {}\n", report.totals.total));
    out.push_str(&format!(
        "- Actual pass/fail: {}/{}\n",
        report.totals.actual_pass, report.totals.actual_fail
    ));
    out.push_str(&format!(
        "- Expected pass/fail: {}/{}\n",
        report.totals.expected_pass, report.totals.expected_fail
    ));
    out.push_str(&format!(
        "- Unexpected status mismatches: {}\n",
        report.totals.unexpected_status_mismatches
    ));
    out.push_str(&format!(
        "- Generate-build fixtures: {}\n\n",
        report.totals.generate_build
    ));

    out.push_str("## Diagnostic code frequency\n\n");
    push_frequency_table(&mut out, &report.diagnostic_code_frequency);
    out.push_str("\n## Coverage tag frequency\n\n");
    push_frequency_table(&mut out, &report.coverage_tag_frequency);
    out.push_str("\n## Entries\n\n");
    out.push_str("| id | kind | expected | actual | diagnostics | tags |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for entry in &report.entries {
        let expected = match entry.expected_check {
            ExpectedCheck::Pass => "pass",
            ExpectedCheck::Fail => "fail",
        };
        let actual = if entry.actual_success { "pass" } else { "fail" };
        out.push_str(&format!(
            "| `{}` | `{}` | {} | {} | {} | {} |\n",
            entry.id,
            entry.fixture_kind.as_str(),
            expected,
            actual,
            markdown_list(&entry.actual_diagnostics),
            markdown_list(&entry.coverage_tags)
        ));
    }
    out
}

fn push_frequency_table(out: &mut String, frequencies: &BTreeMap<String, usize>) {
    if frequencies.is_empty() {
        out.push_str("_None._\n");
        return;
    }
    out.push_str("| item | count |\n");
    out.push_str("| --- | ---: |\n");
    for (item, count) in frequencies {
        out.push_str(&format!("| `{item}` | {count} |\n"));
    }
}

fn markdown_list(items: &[String]) -> String {
    if items.is_empty() {
        "—".to_string()
    } else {
        items
            .iter()
            .map(|item| format!("`{item}`"))
            .collect::<Vec<_>>()
            .join(", ")
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
