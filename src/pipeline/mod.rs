//! Internal generation pipeline orchestration.
//!
//! This module is intentionally crate-internal. It provides a seam between CLI
//! argument handling and the current spec/progenitor/render implementation
//! without committing to a public library API.

use crate::backend::{ApiBackend, ApiCrateRequest, BackendDiagnostic, ProgenitorBackend};
use crate::model::ApiModel;
use crate::render::WrapperManifest;
use crate::spec::{
    report::ReportEntry, transform::TransformPlan, AuthKind, LoadOptions, SpecFacts,
};
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

#[derive(Debug, Clone)]
pub(crate) struct GenerateRequest {
    pub spec_path: PathBuf,
    pub output_path: PathBuf,
    pub bin_name: Option<String>,
    pub base_url: Option<String>,
    pub validate: bool,
    pub load_options: LoadOptions,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct GenerateResult {
    pub facts: SpecFacts,
    pub reports: Vec<ReportEntry>,
    pub transform_plan: TransformPlan,
    pub formatted_warnings: Vec<String>,
    pub backend_diagnostics: Vec<BackendDiagnostic>,
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
    progress: impl FnMut(GenerateProgress),
) -> Result<GenerateResult> {
    let backend = ProgenitorBackend;
    generate_with_backend_and_progress(request, &backend, progress)
}

pub(crate) fn generate_with_backend_and_progress<B: ApiBackend>(
    request: GenerateRequest,
    backend: &B,
    mut progress: impl FnMut(GenerateProgress),
) -> Result<GenerateResult> {
    progress(GenerateProgress::Inspecting {
        spec_path: request.spec_path.clone(),
    });

    let load_options = request
        .load_options
        .with_backend_capabilities(backend.capabilities());
    let loaded = crate::spec::load_with_options(&request.spec_path, &load_options)?;

    for report in &loaded.reports {
        progress(GenerateProgress::Warning {
            warning: report.formatted_warning().to_string(),
        });
    }

    let transform_plan = loaded.transform_plan.clone();
    write_transform_plan(&request.output_path, &transform_plan)?;
    let facts = loaded.facts;
    let target_bin_name = request.bin_name.unwrap_or_else(|| facts.bin_name.clone());
    let api_name = format!("{target_bin_name}-api");

    progress(GenerateProgress::SpecOk {
        operation_count: facts.operation_count,
        auth_kind: facts.auth_kind.clone(),
        target_bin_name: target_bin_name.clone(),
    });

    let (base_url, base_url_is_relative) = effective_base_url(
        request.base_url.as_deref(),
        facts.base_url.as_deref(),
        facts.base_url_is_relative,
    )?;
    let manifest = WrapperManifest::new(
        target_bin_name.clone(),
        base_url,
        base_url_is_relative,
        facts.auth_kind.clone(),
        api_name.clone(),
    );
    let api_model = ApiModel::from_openapi(&loaded.api, manifest.auth_env_var.as_deref())?;
    let manifest = manifest.with_api_model(api_model);

    if let AuthKind::QueryApiKey { param_name } = &manifest.auth_kind {
        progress(GenerateProgress::QueryApiKeyAutoInjectionLimited {
            param_name: param_name.clone(),
        });
    }

    progress(GenerateProgress::GeneratingApiCrate);
    let api_out_dir = request.output_path.join("api");
    let api_output = backend
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
        transform_plan,
        formatted_warnings: loaded.normalization_warnings,
        backend_diagnostics: api_output.diagnostics,
        output_path: request.output_path,
        target_bin_name,
        validation,
    })
}

fn write_transform_plan(output_path: &Path, transform_plan: &TransformPlan) -> Result<()> {
    std::fs::create_dir_all(output_path)
        .with_context(|| format!("failed to create output dir: {}", output_path.display()))?;
    let path = output_path.join("pp-transform-plan.json");
    let body =
        serde_json::to_vec_pretty(transform_plan).context("failed to serialize transform plan")?;
    std::fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))
}

fn effective_base_url(
    explicit: Option<&str>,
    spec_base_url: Option<&str>,
    spec_base_url_is_relative: bool,
) -> Result<(String, bool)> {
    if let Some(base_url) = explicit {
        let is_relative = !(base_url.starts_with("http://") || base_url.starts_with("https://"));
        return Ok((base_url.to_string(), is_relative));
    }
    let Some(base_url) = spec_base_url else {
        return Err(anyhow!(
            "spec has no servers[0].url; pass --base-url explicitly because pp no longer falls back to http://localhost"
        ));
    };
    Ok((base_url.to_string(), spec_base_url_is_relative))
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
    use crate::backend::{
        ApiCrateOutput, BackendCapabilities, SourceTransformDiagnostic, SourceTransformPurpose,
    };
    use crate::spec::normalization_rules::typed;

    const MINIMAL_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Pipeline Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
"#;

    const DEEP_OBJECT_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Backend Capability Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /search:
    get:
      operationId: searchThings
      parameters:
        - name: filter
          in: query
          required: true
          style: deepObject
          schema:
            type: object
            properties:
              name:
                type: string
      responses:
        '200':
          description: ok
"#;

    const MISSING_OPERATION_ID_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Missing Operation ID Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /pets/{id}:
    get:
      responses:
        '200':
          description: ok
"#;

    const MIXED_OPERATION_ID_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Mixed Operation ID Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          description: ok
  /pets/{id}:
    get:
      responses:
        '200':
          description: ok
"#;

    #[test]
    fn generate_returns_result_and_writes_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = write_minimal_spec(temp.path());
        let output_path = temp.path().join("out");

        let backend = FakeBackend::with_diagnostics(vec![fake_source_transform_diagnostic()]);

        let result = generate_with_backend_and_progress(
            GenerateRequest {
                spec_path,
                output_path: output_path.clone(),
                bin_name: Some("fixture-cli".to_string()),
                base_url: None,
                validate: false,
                load_options: LoadOptions::default(),
            },
            &backend,
            |_| {},
        )
        .expect("generate succeeds");

        assert_eq!(result.facts.operation_count, 1);
        assert_eq!(result.target_bin_name, "fixture-cli");
        assert_eq!(result.output_path, output_path);
        assert!(result.validation.is_none());
        assert_eq!(
            result.backend_diagnostics,
            vec![fake_source_transform_diagnostic()]
        );
        assert!(result.output_path.join("Cargo.toml").exists());
        assert!(result.output_path.join("api/src/lib.rs").exists());
        assert!(result.output_path.join("pp-transform-plan.json").exists());
    }

    #[test]
    fn generate_rejects_selected_operation_missing_operation_id_before_backend() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = temp.path().join("spec.yaml");
        std::fs::write(&spec_path, MISSING_OPERATION_ID_SPEC).expect("write spec");
        let output_path = temp.path().join("out");
        let backend = FakeBackend::default();

        let error = generate_with_backend_and_progress(
            GenerateRequest {
                spec_path,
                output_path,
                bin_name: Some("fixture-cli".to_string()),
                base_url: None,
                validate: false,
                load_options: LoadOptions::default(),
            },
            &backend,
            |_| {},
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("operation GET /pets/{id} is missing operationId"));
        assert!(error.contains("explicit operationId is required for codegen/MCP identity"));
    }

    #[test]
    fn generate_succeeds_when_missing_operation_id_operation_is_excluded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = temp.path().join("spec.yaml");
        std::fs::write(&spec_path, MIXED_OPERATION_ID_SPEC).expect("write spec");
        let output_path = temp.path().join("out");
        let backend = FakeBackend::default();

        let result = generate_with_backend_and_progress(
            GenerateRequest {
                spec_path,
                output_path,
                bin_name: Some("fixture-cli".to_string()),
                base_url: None,
                validate: false,
                load_options: LoadOptions {
                    slice: crate::spec::slice::SliceOptions {
                        exclude_operations: vec!["get /pets/{id}".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
            &backend,
            |_| {},
        )
        .expect("excluded unnamed operation is not selected for generation");

        assert_eq!(result.facts.operation_count, 1);
    }

    #[test]
    fn generate_uses_backend_capabilities_during_spec_preparation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = temp.path().join("spec.yaml");
        std::fs::write(&spec_path, DEEP_OBJECT_SPEC).expect("write spec");
        let output_path = temp.path().join("out");
        let mut capabilities = BackendCapabilities::progenitor();
        capabilities.supports_deep_object_query_parameters = true;
        let backend = FakeBackend::with_capabilities(capabilities);

        let result = generate_with_backend_and_progress(
            GenerateRequest {
                spec_path,
                output_path,
                bin_name: Some("fixture-cli".to_string()),
                base_url: None,
                validate: false,
                load_options: LoadOptions::default(),
            },
            &backend,
            |_| {},
        )
        .expect("backend capability avoids deepObject compatibility rewrite");

        assert!(result
            .reports
            .iter()
            .all(|report| report.code != typed::DEEP_OBJECT_QUERY_PARAMS_REWRITTEN));
    }

    #[test]
    fn generate_progress_events_preserve_cli_message_order() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = write_minimal_spec(temp.path());
        let output_path = temp.path().join("out");
        let mut events = Vec::new();

        let backend = FakeBackend::default();

        generate_with_backend_and_progress(
            GenerateRequest {
                spec_path: spec_path.clone(),
                output_path: output_path.clone(),
                bin_name: Some("fixture-cli".to_string()),
                base_url: None,
                validate: false,
                load_options: LoadOptions::default(),
            },
            &backend,
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

    fn fake_source_transform_diagnostic() -> BackendDiagnostic {
        BackendDiagnostic::SourceTransform(SourceTransformDiagnostic {
            name: "fake_transform",
            changed: true,
            replacement_count: 2,
            purpose: SourceTransformPurpose::ClapParserCompatibility,
            precondition: "fake precondition",
            upstream_assumption: "fake upstream assumption",
            upstream_version: "fake upstream version",
        })
    }

    fn write_minimal_spec(dir: &std::path::Path) -> PathBuf {
        let spec_path = dir.join("spec.yaml");
        std::fs::write(&spec_path, MINIMAL_SPEC).expect("write spec");
        spec_path
    }

    struct FakeBackend {
        output: ApiCrateOutput,
        capabilities: BackendCapabilities,
    }

    impl Default for FakeBackend {
        fn default() -> Self {
            Self {
                output: ApiCrateOutput::default(),
                capabilities: BackendCapabilities::progenitor(),
            }
        }
    }

    impl FakeBackend {
        fn with_diagnostics(diagnostics: Vec<BackendDiagnostic>) -> Self {
            Self {
                output: ApiCrateOutput { diagnostics },
                capabilities: BackendCapabilities::progenitor(),
            }
        }

        fn with_capabilities(capabilities: BackendCapabilities) -> Self {
            Self {
                output: ApiCrateOutput::default(),
                capabilities,
            }
        }
    }

    impl ApiBackend for FakeBackend {
        fn name(&self) -> &'static str {
            "fake"
        }

        fn capabilities(&self) -> BackendCapabilities {
            self.capabilities.clone()
        }

        fn generate_api_crate(&self, request: ApiCrateRequest<'_>) -> Result<ApiCrateOutput> {
            assert_eq!(request.crate_name, "fixture-cli-api");
            assert_eq!(request.api.paths.paths.len(), 1);

            std::fs::create_dir_all(request.out_dir.join("src"))?;
            std::fs::write(
                request.out_dir.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                    request.crate_name
                ),
            )?;
            std::fs::write(
                request.out_dir.join("src/lib.rs"),
                "pub fn fake_backend_marker() {}\n",
            )?;

            Ok(self.output.clone())
        }
    }
}
