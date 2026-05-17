//! Render the wrapper crate around progenitor's generated API crate.

use crate::model::{
    ApiModel, McpArgBinding, McpInvocationAdapterContract, McpResponseShaping,
    McpTool as ModelMcpTool,
};
use crate::spec::AuthKind;
use anyhow::{Context, Result};
use heck::ToShoutySnakeCase;
use minijinja::Environment;
use serde::Serialize;
use std::fs;
use std::path::Path;

const CARGO_TEMPLATE: &str = include_str!("templates/Cargo.toml.j2");
const API_CARGO_TEMPLATE: &str = include_str!("templates/api_cargo.toml.j2");
const MAIN_TEMPLATE: &str = include_str!("templates/main.rs.j2");
const CLI_BUILDER_TEMPLATE: &str = include_str!("templates/cli_builder.rs.j2");
const CONTEXT_TEMPLATE: &str = include_str!("templates/context.rs.j2");
const AUTH_TEMPLATE: &str = include_str!("templates/auth.rs.j2");
const INVOKE_TEMPLATE: &str = include_str!("templates/invoke.rs.j2");
const PRINT_TEMPLATE: &str = include_str!("templates/print.rs.j2");
const RUNTIME_TEMPLATE: &str = include_str!("templates/runtime.rs.j2");
const MCP_TEMPLATE: &str = include_str!("templates/mcp.rs.j2");

/// Facts required to render the generated wrapper crate.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct WrapperManifest {
    pub bin_name: String,
    pub base_url: String,
    pub base_url_is_relative: bool,
    pub auth_kind: AuthKind,
    pub progenitor_lib_name: String,
    pub progenitor_crate_name: String,
    pub token_env_var: String,
    pub api_key_env_var: String,
    pub basic_user_env_var: String,
    pub basic_password_env_var: String,
    pub auth_env_var: Option<String>,
    pub mcp_tools: Vec<RenderMcpTool>,
    pub mcp_runtime: McpRuntimeManifest,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct McpRuntimeManifest {
    pub tools_page_size: usize,
    pub temp_body_file_prefix: String,
    pub temp_body_file_prefix_literal: String,
    pub auth_missing_env_literal: Option<String>,
    pub invocation_adapter_kind: String,
    pub invocation_adapter_reason: String,
    pub invocation_adapter: RenderMcpInvocationAdapterContract,
    pub response_shaping: RenderMcpResponseShaping,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RenderMcpInvocationAdapterContract {
    pub kind: String,
    pub kind_literal: String,
    pub kind_rust_variant: String,
    pub reason: String,
    pub reason_literal: String,
    pub direct_typed_invocation: String,
    pub direct_typed_invocation_literal: String,
    pub direct_typed_invocation_rust_variant: String,
    pub requires_generated_cli_command: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RenderMcpTool {
    pub name: String,
    pub name_literal: String,
    pub schema_fn_name: String,
    pub args_static_name: String,
    pub description: String,
    pub description_literal: String,
    pub input_schema: String,
    pub input_schema_literal: String,
    pub args: Vec<RenderMcpArg>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RenderMcpArg {
    pub json_name: String,
    pub json_name_literal: String,
    pub binding_expr: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RenderMcpResponseShaping {
    pub field_filter: RenderMcpResponseShapingArg,
    pub compact: RenderMcpResponseShapingArg,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RenderMcpResponseShapingArg {
    pub json_name_literal: String,
    pub invalid_type_message_literal: String,
}

impl WrapperManifest {
    /// Build template data from inspected facts and the selected progenitor crate name.
    pub(crate) fn new(
        bin_name: String,
        base_url: String,
        base_url_is_relative: bool,
        auth_kind: AuthKind,
        progenitor_lib_name: String,
    ) -> Self {
        let env_prefix = bin_name.to_shouty_snake_case();
        let progenitor_crate_name = progenitor_lib_name.replace('-', "_");
        let auth_env_var = auth_env_var(&auth_kind, &env_prefix);
        let temp_body_file_prefix = format!("{bin_name}-mcp");
        let invocation_adapter =
            render_invocation_adapter(McpInvocationAdapterContract::progenitor_cli_bridge());
        Self {
            bin_name,
            base_url,
            base_url_is_relative,
            auth_kind: auth_kind.clone(),
            progenitor_lib_name,
            progenitor_crate_name,
            token_env_var: format!("{env_prefix}_TOKEN"),
            api_key_env_var: format!("{env_prefix}_API_KEY"),
            basic_user_env_var: format!("{env_prefix}_USER"),
            basic_password_env_var: format!("{env_prefix}_PASSWORD"),
            auth_env_var: auth_env_var.clone(),
            mcp_tools: Vec::new(),
            mcp_runtime: McpRuntimeManifest {
                tools_page_size: 100,
                temp_body_file_prefix_literal: serde_json::to_string(&temp_body_file_prefix)
                    .expect("temp body file prefix serializes"),
                temp_body_file_prefix,
                auth_missing_env_literal: auth_env_var
                    .as_ref()
                    .map(|env| serde_json::to_string(env).expect("auth env var serializes")),
                invocation_adapter_kind: invocation_adapter.kind.clone(),
                invocation_adapter_reason: invocation_adapter.reason.clone(),
                invocation_adapter,
                response_shaping: render_response_shaping(McpResponseShaping::default()),
            },
        }
    }

    pub(crate) fn with_api_model(mut self, api_model: ApiModel) -> Self {
        self.mcp_tools = render_mcp_tools(api_model.mcp_tools);
        self.mcp_runtime.response_shaping = render_response_shaping(api_model.mcp_response_shaping);
        let invocation_adapter = render_invocation_adapter(api_model.mcp_invocation_adapter);
        self.mcp_runtime.invocation_adapter_kind = invocation_adapter.kind.clone();
        self.mcp_runtime.invocation_adapter_reason = invocation_adapter.reason.clone();
        self.mcp_runtime.invocation_adapter = invocation_adapter;
        self
    }
}

fn render_mcp_tools(tools: Vec<ModelMcpTool>) -> Vec<RenderMcpTool> {
    tools
        .into_iter()
        .enumerate()
        .map(|(index, tool)| {
            let tool_index = index + 1;
            RenderMcpTool {
                name_literal: serde_json::to_string(&tool.name).expect("tool name serializes"),
                schema_fn_name: format!("schema_{tool_index}"),
                args_static_name: format!("ARGS_{tool_index}"),
                description_literal: serde_json::to_string(&tool.description)
                    .expect("description serializes"),
                input_schema_literal: serde_json::to_string(&tool.input_schema)
                    .expect("schema literal serializes"),
                args: tool.args.into_iter().map(render_mcp_arg).collect(),
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
            }
        })
        .collect()
}

fn render_mcp_arg(arg: crate::model::McpArg) -> RenderMcpArg {
    let binding_expr = match arg.binding {
        McpArgBinding::CliFlag { cli_name } => {
            let cli_name_literal = serde_json::to_string(&cli_name).expect("arg name serializes");
            format!("crate::invoke::ArgBinding::CliFlag {{ cli_name: {cli_name_literal} }}")
        }
        McpArgBinding::FlattenedBodyField => {
            "crate::invoke::ArgBinding::FlattenedJsonBodyField".to_string()
        }
        McpArgBinding::WholeJsonBody => "crate::invoke::ArgBinding::WholeJsonBody".to_string(),
    };
    RenderMcpArg {
        json_name_literal: serde_json::to_string(&arg.json_name).expect("arg name serializes"),
        json_name: arg.json_name,
        binding_expr,
    }
}

fn render_invocation_adapter(
    adapter: McpInvocationAdapterContract,
) -> RenderMcpInvocationAdapterContract {
    let kind = adapter.kind.as_str().to_string();
    let direct_typed_invocation = adapter.direct_typed_invocation.as_str().to_string();
    RenderMcpInvocationAdapterContract {
        kind_literal: serde_json::to_string(&kind).expect("invocation adapter kind serializes"),
        kind_rust_variant: adapter.kind.rust_variant().to_string(),
        reason_literal: serde_json::to_string(&adapter.reason)
            .expect("invocation adapter reason serializes"),
        direct_typed_invocation_literal: serde_json::to_string(&direct_typed_invocation)
            .expect("direct typed invocation status serializes"),
        direct_typed_invocation_rust_variant: adapter
            .direct_typed_invocation
            .rust_variant()
            .to_string(),
        requires_generated_cli_command: adapter.requires_generated_cli_command,
        kind,
        reason: adapter.reason,
        direct_typed_invocation,
    }
}

fn render_response_shaping(shaping: McpResponseShaping) -> RenderMcpResponseShaping {
    RenderMcpResponseShaping {
        field_filter: RenderMcpResponseShapingArg {
            json_name_literal: serde_json::to_string(&shaping.field_filter.json_name)
                .expect("reserved arg name serializes"),
            invalid_type_message_literal: serde_json::to_string(
                &shaping.field_filter.invalid_type_message,
            )
            .expect("reserved arg message serializes"),
        },
        compact: RenderMcpResponseShapingArg {
            json_name_literal: serde_json::to_string(&shaping.compact.json_name)
                .expect("reserved arg name serializes"),
            invalid_type_message_literal: serde_json::to_string(
                &shaping.compact.invalid_type_message,
            )
            .expect("reserved arg message serializes"),
        },
    }
}

/// Render all wrapper files into `out_dir`.
pub(crate) fn render(manifest: &WrapperManifest, out_dir: &Path) -> Result<()> {
    fs::create_dir_all(out_dir.join("src"))
        .with_context(|| format!("failed to create wrapper src dir: {}", out_dir.display()))?;

    write_template(
        "Cargo.toml",
        CARGO_TEMPLATE,
        manifest,
        &out_dir.join("Cargo.toml"),
    )?;
    write_template(
        "api/Cargo.toml",
        API_CARGO_TEMPLATE,
        manifest,
        &out_dir.join("api/Cargo.toml"),
    )?;
    write_template(
        "main.rs",
        MAIN_TEMPLATE,
        manifest,
        &out_dir.join("src/main.rs"),
    )?;
    write_template(
        "cli_builder.rs",
        CLI_BUILDER_TEMPLATE,
        manifest,
        &out_dir.join("src/cli_builder.rs"),
    )?;
    write_template(
        "context.rs",
        CONTEXT_TEMPLATE,
        manifest,
        &out_dir.join("src/context.rs"),
    )?;
    write_template(
        "auth.rs",
        AUTH_TEMPLATE,
        manifest,
        &out_dir.join("src/auth.rs"),
    )?;
    write_template(
        "invoke.rs",
        INVOKE_TEMPLATE,
        manifest,
        &out_dir.join("src/invoke.rs"),
    )?;
    write_template(
        "print.rs",
        PRINT_TEMPLATE,
        manifest,
        &out_dir.join("src/print.rs"),
    )?;
    write_template(
        "runtime.rs",
        RUNTIME_TEMPLATE,
        manifest,
        &out_dir.join("src/runtime.rs"),
    )?;
    write_template(
        "mcp.rs",
        MCP_TEMPLATE,
        manifest,
        &out_dir.join("src/mcp.rs"),
    )?;
    Ok(())
}

fn render_template(name: &str, source: &str, manifest: &WrapperManifest) -> Result<String> {
    let mut env = Environment::new();
    env.add_template(name, source)?;
    Ok(env.get_template(name)?.render(manifest)?)
}

fn write_template(name: &str, source: &str, manifest: &WrapperManifest, path: &Path) -> Result<()> {
    let rendered = render_template(name, source, manifest)?;
    fs::write(path, rendered).with_context(|| format!("failed to write {}", path.display()))
}

fn auth_env_var(auth_kind: &AuthKind, env_prefix: &str) -> Option<String> {
    match auth_kind {
        AuthKind::None | AuthKind::QueryApiKey { .. } | AuthKind::Unsupported { .. } => None,
        AuthKind::Bearer => Some(format!("{env_prefix}_TOKEN")),
        AuthKind::ApiKey { .. } => Some(format!("{env_prefix}_API_KEY")),
        AuthKind::HttpBasic => Some(format!("{env_prefix}_USER/{env_prefix}_PASSWORD")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_template_uses_manifest_runtime_metadata() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Metadata API
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: page
          in: query
          schema:
            type: integer
      responses:
        '200':
          description: ok
"#;
        let api: openapiv3::OpenAPI = serde_yaml::from_str(spec).unwrap();
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::Bearer,
            "petstore-api".to_string(),
        );
        let api_model = ApiModel::from_openapi(&api, manifest.auth_env_var.as_deref()).unwrap();
        let manifest = manifest.with_api_model(api_model);

        let rendered = render_template("mcp.rs", MCP_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("const TOOLS_PAGE_SIZE: usize = 100;"));
        assert!(rendered.contains("fn schema_1() -> rmcp::model::JsonObject"));
        assert!(rendered.contains("static ARGS_1: &[ArgDef]"));
        assert!(rendered.contains("crate::invoke::ArgBinding::"));
        assert!(rendered.contains("name: \"list_items\""));
        assert!(rendered.contains("crate::runtime::parse_response_shaping(&arguments)"));
        assert!(rendered.contains("crate::runtime::classify_tool_error(result.value)"));
        assert!(rendered.contains("response_shaping.shape_success(result.value)"));
        assert!(!rendered.contains("fn parse_field_filter"));
        assert!(!rendered.contains("fn classify_tool_error"));
        assert!(rendered.contains("\"env\": \"PETSTORE_TOKEN\""));
        assert!(rendered.contains("invoke_operation("));
        assert!(rendered.contains("validate_mcp_invocation_bridge()"));
        assert!(!rendered.contains("write_json_body"));
    }

    #[test]
    fn runtime_template_owns_response_shaping_helpers() {
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
            "petstore-api".to_string(),
        );

        let rendered = render_template("runtime.rs", RUNTIME_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("pub fn parse_response_shaping("));
        assert!(rendered.contains("fn parse_field_filter("));
        assert!(rendered.contains("fn compact_value("));
        assert!(rendered.contains("pub fn classify_tool_error("));
        assert!(rendered.contains("arguments.get(\"_pp_fields\")"));
        assert!(rendered.contains("McpError::invalid_params(\"_pp_compact must be a boolean\""));
    }

    #[test]
    fn mcp_template_avoids_arg_binding_import_for_no_arg_tools() {
        let spec = r#"
openapi: 3.0.0
info:
  title: No Args API
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      responses:
        '200':
          description: ok
"#;
        let api: openapiv3::OpenAPI = serde_yaml::from_str(spec).unwrap();
        let manifest = WrapperManifest::new(
            "noargs".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
            "noargs-api".to_string(),
        );
        let api_model = ApiModel::from_openapi(&api, None).unwrap();
        let manifest = manifest.with_api_model(api_model);

        let rendered = render_template("mcp.rs", MCP_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("static ARGS_1: &[ArgDef] = &["));
        assert!(!rendered.contains("use crate::invoke::{invoke_operation, ArgBinding"));
        assert!(!rendered.contains("ArgBinding::"));
    }

    #[test]
    fn mcp_template_avoids_value_import_for_zero_tool_servers() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Empty API
  version: "1.0.0"
paths: {}
"#;
        let api: openapiv3::OpenAPI = serde_yaml::from_str(spec).unwrap();
        let manifest = WrapperManifest::new(
            "empty".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
            "empty-api".to_string(),
        );
        let api_model = ApiModel::from_openapi(&api, None).unwrap();
        let manifest = manifest.with_api_model(api_model);

        let rendered = render_template("mcp.rs", MCP_TEMPLATE, &manifest).unwrap();

        assert!(!rendered.contains("use serde_json::Value"));
        assert!(!rendered.contains("fn schema_"));
    }

    #[test]
    fn invocation_template_uses_process_local_counter_for_temp_body_files() {
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
            "petstore-api".to_string(),
        );

        let rendered = render_template("invoke.rs", INVOKE_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("pub async fn invoke_operation"));
        assert!(rendered.contains("struct ProgenitorCliBridgeInvoker"));
        assert!(rendered.contains("pub enum InvocationAdapterKind"));
        assert!(rendered.contains("pub enum DirectTypedInvocationStatus"));
        assert!(rendered.contains("pub struct InvocationAdapterContract"));
        assert!(rendered.contains("trait OperationInvoker"));
        assert!(rendered.contains(
            "pub const INVOCATION_ADAPTER_KIND: InvocationAdapterKind = INVOCATION_ADAPTER_CONTRACT.kind;"
        ));
        assert!(rendered.contains(
            "pub const INVOCATION_ADAPTER_KIND_NAME: &str = INVOCATION_ADAPTER_KIND.as_str();"
        ));
        assert!(rendered.contains("pub const DIRECT_TYPED_INVOCATION_STATUS_NAME: &str = DIRECT_TYPED_INVOCATION_STATUS.as_str();"));
        assert!(rendered.contains("validate_invocation_adapter_contract_for_tool"));
        assert!(rendered.contains("requires_generated_cli_command: true"));
        assert!(rendered.contains("Adapter contract:"));
        assert!(rendered.contains("generated CLI argv/Clap dispatch semantics"));
        assert!(!rendered.contains("backend debt"));
        assert!(!rendered.contains("Transitional Progenitor CLI bridge"));
        assert_eq!(
            manifest.mcp_runtime.invocation_adapter_kind,
            "progenitor_cli_bridge"
        );
        assert_eq!(
            manifest.mcp_runtime.invocation_adapter.kind_rust_variant,
            "InvocationAdapterKind::ProgenitorCliBridge"
        );
        assert_eq!(
            manifest
                .mcp_runtime
                .invocation_adapter
                .direct_typed_invocation,
            "unsupported"
        );
        assert!(
            manifest
                .mcp_runtime
                .invocation_adapter
                .requires_generated_cli_command
        );
        assert!(manifest
            .mcp_runtime
            .invocation_adapter_reason
            .contains("direct typed operation invocation is not supported"));
        assert!(rendered.contains(".create_new(true)"));
        assert!(rendered.contains("static MCP_BODY_FILE_COUNTER: AtomicU64"));
        assert!(rendered.contains("MCP_BODY_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)"));
        assert!(rendered.contains("{}-{}-{}-{}-body.json"));
        assert!(rendered.contains("\"petstore-mcp\""));
    }

    #[test]
    fn cargo_template_contains_workspace_and_api_dependency() {
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
            "petstore-api".to_string(),
        );

        let rendered = render_template("Cargo.toml", CARGO_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("[workspace]"));
        assert!(rendered.contains("members = [\"api\", \".\"]"));
        assert!(rendered.contains("name = \"petstore\""));
        assert!(rendered.contains("petstore-api = { path = \"api\" }"));
    }
}
