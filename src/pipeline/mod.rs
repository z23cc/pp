//! Internal generation pipeline orchestration.
//!
//! This module is intentionally crate-internal. It provides a seam between CLI
//! argument handling, strict OpenAPI inspection, and native wrapper rendering
//! without committing to a public library API.

use crate::backend::{ApiBackend, NativeHttpBackend};
use crate::model::ApiModel;
use crate::render::WrapperManifest;
use crate::spec::{
    report::ReportEntry,
    transform::{TransformActionKind, TransformAuditEntry, TransformPlan},
    AuthKind, LoadOptions, SpecFacts,
};
use anyhow::{anyhow, Context, Result};
use serde_json::json;
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
    QueryApiKeyUsesExplicitParameter {
        param_name: String,
    },
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
    let backend = NativeHttpBackend;
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

    let backend_capabilities = backend.capabilities();
    let loaded = crate::spec::load_with_options(&request.spec_path, &request.load_options)?;

    for report in &loaded.reports {
        progress(GenerateProgress::Warning {
            warning: report.formatted_warning().to_string(),
        });
    }

    let mut transform_plan = loaded.transform_plan.clone();
    write_transform_plan(&request.output_path, &transform_plan)?;
    let facts = loaded.facts;
    let target_bin_name = request.bin_name.unwrap_or_else(|| facts.bin_name.clone());
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
    );
    let api_model = ApiModel::from_spec_with_direct_invocation(
        &loaded.spec,
        manifest.auth_env_var.as_deref(),
        &backend_capabilities.direct_invocation,
    )?;
    let manifest = manifest.with_api_model(api_model);
    transform_plan.add_audits(runtime_generation_audits(&manifest, &backend_capabilities));
    write_transform_plan(&request.output_path, &transform_plan)?;
    ensure_no_unsupported_operations(&manifest)?;

    if let AuthKind::QueryApiKey { param_name } = &manifest.auth_kind {
        progress(GenerateProgress::QueryApiKeyUsesExplicitParameter {
            param_name: param_name.clone(),
        });
    }

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
        formatted_warnings: loaded.preparation_warnings,
        output_path: request.output_path,
        target_bin_name,
        validation,
    })
}

fn ensure_no_unsupported_operations(manifest: &WrapperManifest) -> Result<()> {
    if manifest.unsupported_mcp_operations.is_empty() {
        return Ok(());
    }

    let details = manifest
        .unsupported_mcp_operations
        .iter()
        .map(|operation| {
            let operation_id = operation
                .operation_id
                .as_deref()
                .map(|id| format!(" operationId '{id}'"))
                .unwrap_or_default();
            format!(
                "{} {}{}: {}",
                operation.method, operation.path, operation_id, operation.reason
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    Err(anyhow!(
        "unsupported native direct HTTP operation shape(s): {details}. Exclude unsupported operations with --exclude-operation or narrow generation with slice filters."
    ))
}

fn runtime_generation_audits(
    manifest: &WrapperManifest,
    capabilities: &crate::backend::BackendCapabilities,
) -> Vec<TransformAuditEntry> {
    let mut audits = vec![TransformAuditEntry::new(
        "runtime_generation",
        "runtime.mcp_invocation.direct_http",
        "generated src/invoke.rs",
        "route MCP tool calls through the direct HTTP adapter",
    )
    .with_action_kind(TransformActionKind::RuntimeDirectInvocation)
    .with_backend_requirement_id(capabilities.direct_invocation.requirement_id)
    .with_backend_requirement(capabilities.direct_invocation.invocation_requirement)
    .with_before_after(
        "no explicit runtime-generation direct invocation audit",
        manifest.mcp_runtime.invocation_adapter_kind.as_str(),
    )
    .with_before_after_json(
        json!(null),
        json!({
            "invocation_adapter_kind": &manifest.mcp_runtime.invocation_adapter_kind,
            "invocation_adapter_reason": &manifest.mcp_runtime.invocation_adapter_reason,
            "direct_typed_invocation": &manifest.mcp_runtime.invocation_adapter.direct_typed_invocation,
            "requires_generated_cli_command": manifest.mcp_runtime.invocation_adapter.requires_generated_cli_command,
            "backend_profile": capabilities.profile.as_str(),
            "direct_tool_count": manifest.mcp_tools.len(),
            "unsupported_tool_count": manifest.unsupported_mcp_operations.len(),
            "preserves_runtime_behavior": true,
        }),
    )];

    audits.extend(manifest.unsupported_mcp_operations.iter().map(|operation| {
        TransformAuditEntry::new(
            "runtime_generation",
            "runtime.mcp_invocation.unsupported_operation",
            format!("{} {}", operation.method, operation.path),
            "exclude operation from MCP tools/list because direct HTTP invocation is unsupported",
        )
        .with_action_kind(TransformActionKind::RuntimeDirectInvocation)
        .with_backend_requirement_id("mcp.direct_http.supported_operation_shape")
        .with_backend_requirement(
            capabilities
                .direct_invocation
                .supported_operation_requirement,
        )
        .with_before_after(
            "operation selected for generation",
            "operation excluded from MCP tools/list",
        )
        .with_before_after_json(
            json!({
                "operation_id": operation.operation_id,
                "method": operation.method,
                "path": operation.path,
            }),
            json!({
                "reason": operation.reason,
            }),
        )
    }));

    audits
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
    _spec_base_url_is_relative: bool,
) -> Result<(String, bool)> {
    if let Some(base_url) = explicit {
        reject_non_absolute_runtime_base_url(base_url, "--base-url")?;
        return Ok((base_url.to_string(), false));
    }
    let Some(base_url) = spec_base_url else {
        return Err(anyhow!(
            "spec has no servers[0].url; pass --base-url explicitly because pp requires an explicit runtime base URL"
        ));
    };
    reject_non_absolute_runtime_base_url(base_url, "servers[0].url")?;
    Ok((base_url.to_string(), false))
}

fn reject_non_absolute_runtime_base_url(base_url: &str, source: &str) -> Result<()> {
    if base_url.starts_with("http://") || base_url.starts_with("https://") {
        return Ok(());
    }
    Err(anyhow!(
        "{source} must be an absolute http(s) URL for native direct HTTP generation: {base_url}"
    ))
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
    use crate::backend::BackendCapabilities;

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

    const QUERY_ARRAY_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Direct Invocation Capability Fixture
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: tags
          in: query
          schema:
            type: array
            items:
              type: string
      responses:
        '200':
          description: ok
"#;

    const RELATIVE_SERVER_SPEC: &str = r#"
openapi: 3.0.0
info:
  title: Relative Server Fixture
  version: "1.0.0"
servers:
  - url: /api/v1
paths:
  /pets:
    get:
      operationId: listPets
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

        let backend = FakeBackend::default();

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
        assert!(result.output_path.join("Cargo.toml").exists());
        assert!(!result.output_path.join("api").exists());
        let transform_plan_path = result.output_path.join("pp-transform-plan.json");
        assert!(transform_plan_path.exists());
        assert!(result.transform_plan.audits.iter().any(|audit| {
            audit.source_stage == "runtime_generation"
                && audit.code == "runtime.mcp_invocation.direct_http"
                && audit.action_kind == Some(TransformActionKind::RuntimeDirectInvocation)
                && audit.backend_requirement_id.as_deref()
                    == Some("native_http.direct_http.invocation")
                && audit
                    .backend_requirement
                    .as_deref()
                    .is_some_and(|requirement| requirement.contains("native HTTP runtime"))
        }));
        let transform_plan_json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(transform_plan_path).expect("read transform plan"),
        )
        .expect("parse transform plan");
        assert!(transform_plan_json["audits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|audit| {
                audit["source_stage"] == "runtime_generation"
                    && audit["code"] == "runtime.mcp_invocation.direct_http"
                    && audit["action_kind"] == "runtime_direct_invocation"
                    && audit["backend_requirement_id"] == "native_http.direct_http.invocation"
                    && audit["backend_requirement"]
                        .as_str()
                        .unwrap()
                        .contains("native HTTP runtime")
                    && audit["after_json"]["backend_profile"] == "native_http"
                    && audit["after_json"]["invocation_adapter_kind"] == "direct_http"
                    && audit["after_json"]["direct_typed_invocation"] == "supported"
                    && audit["after_json"]["requires_generated_cli_command"] == false
            }));
    }

    #[test]
    fn generate_rejects_relative_spec_server_url() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = temp.path().join("spec.yaml");
        std::fs::write(&spec_path, RELATIVE_SERVER_SPEC).expect("write spec");
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

        assert!(error.contains("servers[0].url must be an absolute http(s) URL"));
    }

    #[test]
    fn generate_rejects_relative_explicit_base_url() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = write_minimal_spec(temp.path());
        let output_path = temp.path().join("out");
        let backend = FakeBackend::default();

        let error = generate_with_backend_and_progress(
            GenerateRequest {
                spec_path,
                output_path,
                bin_name: Some("fixture-cli".to_string()),
                base_url: Some("/api/v1".to_string()),
                validate: false,
                load_options: LoadOptions::default(),
            },
            &backend,
            |_| {},
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("--base-url must be an absolute http(s) URL"));
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
    fn native_generation_uses_backend_capabilities_for_query_array_modeling() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = temp.path().join("spec.yaml");
        std::fs::write(&spec_path, QUERY_ARRAY_SPEC).expect("write spec");
        let output_path = temp.path().join("out");
        let backend = FakeBackend::default();

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
        .expect("native direct invocation capability allows primitive query arrays");

        assert!(result.transform_plan.audits.iter().all(|audit| {
            !(audit.source_stage == "runtime_generation"
                && audit.code == "runtime.mcp_invocation.unsupported_operation"
                && audit.target == "GET /items")
        }));
        assert!(result.transform_plan.audits.iter().any(|audit| {
            audit.source_stage == "runtime_generation"
                && audit.code == "runtime.mcp_invocation.direct_http"
                && audit
                    .after_json
                    .as_ref()
                    .and_then(|after| after.get("direct_tool_count"))
                    == Some(&json!(1))
        }));
    }

    #[test]
    fn generation_rejects_unsupported_deep_object_query_params_without_rewriting_spec() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec_path = temp.path().join("spec.yaml");
        std::fs::write(&spec_path, DEEP_OBJECT_SPEC).expect("write spec");
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

        assert!(error.contains("unsupported native direct HTTP operation shape"));
        assert!(error.contains("object parameter 'filter'"));
        assert!(!error.contains("spec.prepare.deep_object_query_params_rewritten"));
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
            ["inspect", "spec_ok", "render_wrapper", "workspace_written"]
        );
    }

    fn write_minimal_spec(dir: &std::path::Path) -> PathBuf {
        let spec_path = dir.join("spec.yaml");
        std::fs::write(&spec_path, MINIMAL_SPEC).expect("write spec");
        spec_path
    }

    struct FakeBackend {
        capabilities: BackendCapabilities,
    }

    impl Default for FakeBackend {
        fn default() -> Self {
            Self {
                capabilities: BackendCapabilities::native_http(),
            }
        }
    }

    impl ApiBackend for FakeBackend {
        fn capabilities(&self) -> BackendCapabilities {
            self.capabilities.clone()
        }
    }
}
