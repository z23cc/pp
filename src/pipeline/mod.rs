//! Internal generation pipeline orchestration.
//!
//! This module is intentionally crate-internal. It provides a seam between CLI
//! argument handling and the current spec/progenitor/render implementation
//! without committing to a public library API.

use crate::backend::{ApiBackend, ApiCrateRequest, ProgenitorBackend};
use crate::model::ApiModel;
use crate::render::WrapperManifest;
use crate::spec::{report::ReportEntry, AuthKind, LoadOptions, SpecFacts};
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

#[derive(Debug, Clone)]
pub(crate) struct GenerateRequest {
    pub spec_path: PathBuf,
    pub output_path: PathBuf,
    pub bin_name: Option<String>,
    pub validate: bool,
    pub load_options: LoadOptions,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct GenerateResult {
    pub facts: SpecFacts,
    pub reports: Vec<ReportEntry>,
    pub formatted_warnings: Vec<String>,
    pub output_path: PathBuf,
    pub target_bin_name: String,
    pub validation: Option<ValidationResult>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct ValidationResult {
    pub workspace_path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) enum GenerateProgress {
    Inspecting {
        spec_path: PathBuf,
    },
    Warning {
        warning: String,
    },
    SpecOk {
        operation_count: usize,
        auth_kind: AuthKind,
        target_bin_name: String,
    },
    QueryApiKeyAutoInjectionLimited {
        param_name: String,
    },
    GeneratingApiCrate,
    RenderingWrapperCrate,
    WorkspaceWritten {
        output_path: PathBuf,
    },
    BuildStarted,
    BuildSucceeded,
}

#[allow(dead_code)]
pub(crate) fn generate(request: GenerateRequest) -> Result<GenerateResult> {
    generate_with_progress(request, |_| {})
}

pub(crate) fn generate_with_progress(
    request: GenerateRequest,
    mut progress: impl FnMut(GenerateProgress),
) -> Result<GenerateResult> {
    progress(GenerateProgress::Inspecting {
        spec_path: request.spec_path.clone(),
    });

    let loaded = if request.load_options.slice.is_noop() {
        crate::spec::load(&request.spec_path)?
    } else {
        crate::spec::load_with_options(&request.spec_path, &request.load_options)?
    };

    for report in &loaded.reports {
        progress(GenerateProgress::Warning {
            warning: report.formatted_warning().to_string(),
        });
    }

    let facts = loaded.facts;
    let target_bin_name = request.bin_name.unwrap_or_else(|| facts.bin_name.clone());
    let api_name = format!("{target_bin_name}-api");

    progress(GenerateProgress::SpecOk {
        operation_count: facts.operation_count,
        auth_kind: facts.auth_kind.clone(),
        target_bin_name: target_bin_name.clone(),
    });

    let manifest = WrapperManifest::new(
        target_bin_name.clone(),
        facts.base_url.clone(),
        facts.base_url_is_relative,
        facts.auth_kind.clone(),
        api_name.clone(),
    );
    let api_model = ApiModel::from_openapi(&loaded.api, manifest.auth_env_var.as_deref())?;
    let manifest = manifest.with_mcp_tools(api_model.mcp_tools);

    if let AuthKind::QueryApiKey { param_name } = &manifest.auth_kind {
        progress(GenerateProgress::QueryApiKeyAutoInjectionLimited {
            param_name: param_name.clone(),
        });
    }

    progress(GenerateProgress::GeneratingApiCrate);
    let api_out_dir = request.output_path.join("api");
    let backend = ProgenitorBackend;
    backend
        .generate_api_crate(ApiCrateRequest {
            api: &loaded.api,
            out_dir: &api_out_dir,
            crate_name: &api_name,
        })
        .with_context(|| format!("{} backend failed to generate API crate", backend.name()))?;

    progress(GenerateProgress::RenderingWrapperCrate);
    crate::render::render(&manifest, &request.output_path)?;

    progress(GenerateProgress::WorkspaceWritten {
        output_path: request.output_path.clone(),
    });

    let validation = if request.validate {
        progress(GenerateProgress::BuildStarted);
        let validation = validate_workspace_build(&request.output_path)?;
        progress(GenerateProgress::BuildSucceeded);
        Some(validation)
    } else {
        None
    };

    Ok(GenerateResult {
        facts,
        reports: loaded.reports,
        formatted_warnings: loaded.normalization_warnings,
        output_path: request.output_path,
        target_bin_name,
        validation,
    })
}

pub(crate) fn validate_workspace_build(workspace: &Path) -> Result<ValidationResult> {
    let out = ProcessCommand::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(workspace)
        .output()
        .with_context(|| format!("failed to spawn cargo build in {}", workspace.display()))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!(
            "cargo build --release failed (exit {}):\n{stderr}",
            out.status.code().unwrap_or(-1)
        ));
    }
    Ok(ValidationResult {
        workspace_path: workspace.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Pipeline Fixture
  version: "1.0.0"
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
"#;

    #[test]
    fn generate_returns_result_and_writes_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = write_minimal_spec(temp.path());
        let output_path = temp.path().join("out");

        let result = generate(GenerateRequest {
            spec_path,
            output_path: output_path.clone(),
            bin_name: Some("fixture-cli".to_string()),
            validate: false,
            load_options: LoadOptions::default(),
        })
        .expect("generate succeeds");

        assert_eq!(result.facts.operation_count, 1);
        assert_eq!(result.target_bin_name, "fixture-cli");
        assert_eq!(result.output_path, output_path);
        assert!(result.validation.is_none());
        assert!(result.output_path.join("Cargo.toml").exists());
        assert!(result.output_path.join("api/src/lib.rs").exists());
    }

    #[test]
    fn generate_progress_events_preserve_cli_message_order() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = write_minimal_spec(temp.path());
        let output_path = temp.path().join("out");
        let mut events = Vec::new();

        generate_with_progress(
            GenerateRequest {
                spec_path: spec_path.clone(),
                output_path: output_path.clone(),
                bin_name: Some("fixture-cli".to_string()),
                validate: false,
                load_options: LoadOptions::default(),
            },
            |event| match event {
                GenerateProgress::Inspecting { spec_path: path } => {
                    assert_eq!(path, spec_path);
                    events.push("inspect");
                }
                GenerateProgress::SpecOk {
                    operation_count,
                    target_bin_name,
                    ..
                } => {
                    assert_eq!(operation_count, 1);
                    assert_eq!(target_bin_name, "fixture-cli");
                    events.push("spec_ok");
                }
                GenerateProgress::GeneratingApiCrate => events.push("generate_api"),
                GenerateProgress::RenderingWrapperCrate => events.push("render_wrapper"),
                GenerateProgress::WorkspaceWritten { output_path: path } => {
                    assert_eq!(path, output_path);
                    events.push("workspace_written");
                }
                other => panic!("unexpected progress event: {other:?}"),
            },
        )
        .expect("generate succeeds");

        assert_eq!(
            events,
            [
                "inspect",
                "spec_ok",
                "generate_api",
                "render_wrapper",
                "workspace_written"
            ]
        );
    }

    fn write_minimal_spec(dir: &std::path::Path) -> PathBuf {
        let spec_path = dir.join("spec.yaml");
        std::fs::write(&spec_path, MINIMAL_SPEC).expect("write spec");
        spec_path
    }
}
