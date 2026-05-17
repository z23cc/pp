//! Render the generated native HTTP CLI/MCP wrapper crate.

use crate::model::{
    ApiModel, ArgValueKind, McpArgBinding, McpInvocationAdapterContract, McpResponseShaping,
    McpTool as ModelMcpTool, PrimitiveKind,
};
use crate::spec::AuthKind;
use anyhow::{Context, Result};
use heck::ToShoutySnakeCase;
use minijinja::Environment;
use serde::Serialize;
use std::fs;
use std::path::Path;

const CARGO_TEMPLATE: &str = include_str!("templates/Cargo.toml.j2");
const MAIN_TEMPLATE: &str = include_str!("templates/main.rs.j2");
const CLI_BUILDER_TEMPLATE: &str = include_str!("templates/cli_builder.rs.j2");
const CONTEXT_TEMPLATE: &str = include_str!("templates/context.rs.j2");
const AUTH_TEMPLATE: &str = include_str!("templates/auth.rs.j2");
const DIRECT_HTTP_TEMPLATE: &str = include_str!("templates/direct_http.rs.j2");
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
    pub token_env_var: String,
    pub api_key_env_var: String,
    pub basic_user_env_var: String,
    pub basic_password_env_var: String,
    pub auth_env_var: Option<String>,
    pub mcp_tools: Vec<RenderMcpTool>,
    pub unsupported_mcp_operations: Vec<RenderMcpUnsupportedOperation>,
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
pub(crate) struct RenderMcpUnsupportedOperation {
    pub operation_id: Option<String>,
    pub method: String,
    pub path: String,
    pub reason: String,
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
    pub method: String,
    pub method_literal: String,
    pub path_template: String,
    pub path_template_literal: String,
    pub args: Vec<RenderMcpArg>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RenderMcpArg {
    pub json_name: String,
    pub json_name_literal: String,
    pub binding_expr: String,
    pub required_literal: &'static str,
    pub value_kind_expr: String,
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
    /// Build template data from inspected facts and native direct-HTTP metadata.
    pub(crate) fn new(
        bin_name: String,
        base_url: String,
        base_url_is_relative: bool,
        auth_kind: AuthKind,
    ) -> Self {
        let env_prefix = bin_name.to_shouty_snake_case();
        let auth_env_var = auth_env_var(&auth_kind, &env_prefix);
        let temp_body_file_prefix = format!("{bin_name}-mcp");
        let invocation_adapter =
            render_invocation_adapter(McpInvocationAdapterContract::direct_http());
        Self {
            bin_name,
            base_url,
            base_url_is_relative,
            auth_kind: auth_kind.clone(),
            token_env_var: format!("{env_prefix}_TOKEN"),
            api_key_env_var: format!("{env_prefix}_API_KEY"),
            basic_user_env_var: format!("{env_prefix}_USER"),
            basic_password_env_var: format!("{env_prefix}_PASSWORD"),
            auth_env_var: auth_env_var.clone(),
            mcp_tools: Vec::new(),
            unsupported_mcp_operations: Vec::new(),
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
        self.unsupported_mcp_operations = api_model
            .unsupported_mcp_operations
            .into_iter()
            .map(|operation| RenderMcpUnsupportedOperation {
                operation_id: operation.operation_id,
                method: operation.method,
                path: operation.path,
                reason: operation.reason,
            })
            .collect();
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
                method_literal: serde_json::to_string(&tool.method)
                    .expect("method literal serializes"),
                path_template_literal: serde_json::to_string(&tool.path_template)
                    .expect("path template literal serializes"),
                args: tool.args.into_iter().map(render_mcp_arg).collect(),
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
                method: tool.method,
                path_template: tool.path_template,
            }
        })
        .collect()
}

fn render_mcp_arg(arg: crate::model::McpArg) -> RenderMcpArg {
    let binding_expr = match arg.binding {
        McpArgBinding::PathParam { wire_name } => {
            let wire_name_literal = serde_json::to_string(&wire_name).expect("arg name serializes");
            format!("crate::invoke::ArgBinding::PathParam {{ wire_name: {wire_name_literal} }}")
        }
        McpArgBinding::QueryParam { wire_name } => {
            let wire_name_literal = serde_json::to_string(&wire_name).expect("arg name serializes");
            format!("crate::invoke::ArgBinding::QueryParam {{ wire_name: {wire_name_literal} }}")
        }
        McpArgBinding::FlattenedBodyField => {
            "crate::invoke::ArgBinding::FlattenedJsonBodyField".to_string()
        }
        McpArgBinding::WholeJsonBody => "crate::invoke::ArgBinding::WholeJsonBody".to_string(),
    };
    let value_kind_expr = render_arg_value_kind(&arg.value_kind);
    RenderMcpArg {
        json_name_literal: serde_json::to_string(&arg.json_name).expect("arg name serializes"),
        json_name: arg.json_name,
        binding_expr,
        required_literal: if arg.required { "true" } else { "false" },
        value_kind_expr,
    }
}

fn render_arg_value_kind(value_kind: &ArgValueKind) -> String {
    match value_kind {
        ArgValueKind::String => "CliValueKind::String".to_string(),
        ArgValueKind::Integer => "CliValueKind::Integer".to_string(),
        ArgValueKind::Number => "CliValueKind::Number".to_string(),
        ArgValueKind::Boolean => "CliValueKind::Boolean".to_string(),
        ArgValueKind::Json => "CliValueKind::Json".to_string(),
        ArgValueKind::NullablePrimitive { item } => format!(
            "CliValueKind::NullablePrimitive {{ item: {} }}",
            render_primitive_kind(*item)
        ),
        ArgValueKind::PrimitiveArray { item } => format!(
            "CliValueKind::PrimitiveArray {{ item: {} }}",
            render_primitive_kind(*item)
        ),
    }
}

fn render_primitive_kind(kind: PrimitiveKind) -> &'static str {
    match kind {
        PrimitiveKind::String => "CliPrimitiveKind::String",
        PrimitiveKind::Integer => "CliPrimitiveKind::Integer",
        PrimitiveKind::Number => "CliPrimitiveKind::Number",
        PrimitiveKind::Boolean => "CliPrimitiveKind::Boolean",
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
        "direct_http.rs",
        DIRECT_HTTP_TEMPLATE,
        manifest,
        &out_dir.join("src/direct_http.rs"),
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
    fn mcp_template_required_validation_accepts_explicit_null() {
        let manifest = WrapperManifest::new(
            "nullable-api".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
        );

        let rendered = render_template("mcp.rs", MCP_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("if !arguments.contains_key(name)"));
        assert!(!rendered.contains("serde_json::Value::is_null"));
    }

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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::Bearer,
        );
        let api_model = ApiModel::from_spec(&api, manifest.auth_env_var.as_deref()).unwrap();
        let manifest = manifest.with_api_model(api_model);

        let rendered = render_template("mcp.rs", MCP_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("const TOOLS_PAGE_SIZE: usize = 100;"));
        assert!(rendered.contains("fn schema_1() -> rmcp::model::JsonObject"));
        assert!(rendered.contains("static ARGS_1: &[ArgDef]"));
        assert!(rendered.contains("crate::invoke::ArgBinding::"));
        assert!(rendered.contains("name: \"list_items\""));
        assert!(rendered.contains("method: \"GET\""));
        assert!(rendered.contains("path_template: \"/items\""));
        assert!(rendered.contains("crate::runtime::parse_response_shaping(&arguments)"));
        assert!(rendered.contains("crate::runtime::classify_tool_error(result.value)"));
        assert!(rendered.contains("response_shaping.shape_success(result.value)"));
        assert!(!rendered.contains("fn parse_field_filter"));
        assert!(!rendered.contains("fn classify_tool_error"));
        assert!(rendered.contains("\"env\": \"PETSTORE_TOKEN\""));
        assert!(rendered.contains("invoke_operation("));
        assert!(rendered.contains("validate_mcp_direct_invocation()"));
        assert!(!rendered.contains("write_json_body"));
    }

    #[test]
    fn cli_builder_template_uses_native_operation_dispatch() {
        let spec = r#"
openapi: 3.0.0
info:
  title: CLI API
  version: "1.0.0"
paths:
  /items/{id}:
    post:
      operationId: createItem
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: integer
        - name: tag
          in: query
          schema:
            type: array
            items:
              type: string
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              required: [name]
              properties:
                name:
                  type: string
                active:
                  type: boolean
      responses:
        '200':
          description: ok
  /bulk:
    post:
      operationId: replaceItems
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: array
              items:
                type: string
      responses:
        '200':
          description: ok
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let manifest = WrapperManifest::new(
            "cli-api".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
        );
        let api_model = ApiModel::from_spec(&api, None).unwrap();
        let manifest = manifest.with_api_model(api_model);

        let rendered = render_template("cli_builder.rs", CLI_BUILDER_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("static OPERATIONS: &[CliOperationDef]"));
        assert!(rendered.contains("invoke_operation("));
        assert!(rendered.contains("CliValueKind::Integer"));
        assert!(
            rendered.contains("CliValueKind::PrimitiveArray { item: CliPrimitiveKind::String }")
        );
        assert!(rendered.contains("CliValueKind::Boolean"));
        assert!(rendered.contains("CliValueKind::Json"));
        assert!(rendered
            .contains("json_name: \"body\", required: true, value_kind: CliValueKind::Json"));
        assert!(rendered.contains("clap::ArgAction::Append"));
        assert!(rendered.contains("crate::print::emit_cli_success"));
    }

    #[test]
    fn print_template_uses_native_output_helpers() {
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
        );

        let rendered = render_template("print.rs", PRINT_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("pub fn emit_cli_success"));
        assert!(rendered.contains("pub fn emit_cli_error"));
    }

    #[test]
    fn direct_http_template_owns_generic_http_helpers() {
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
        );

        let rendered = render_template("direct_http.rs", DIRECT_HTTP_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("pub(crate) fn build_request_parts("));
        assert!(rendered.contains("pub(crate) fn build_url("));
        assert!(rendered.contains("fn collect_query_pairs("));
        assert!(rendered.contains("fn encode_path_value("));
        assert!(rendered.contains("ArgBinding::PathParam"));
        assert!(rendered.contains("ArgBinding::QueryParam"));
        assert!(rendered.contains("pub(crate) fn headers_to_json("));
        assert!(rendered.contains("pub(crate) fn transport_error("));
        assert!(rendered.contains("pub(crate) fn success_response("));
        assert!(rendered.contains("pub(crate) fn error_response("));
    }

    #[test]
    fn runtime_template_owns_response_shaping_helpers() {
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let manifest = WrapperManifest::new(
            "noargs".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
        );
        let api_model = ApiModel::from_spec(&api, None).unwrap();
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let manifest = WrapperManifest::new(
            "empty".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
        );
        let api_model = ApiModel::from_spec(&api, None).unwrap();
        let manifest = manifest.with_api_model(api_model);

        let rendered = render_template("mcp.rs", MCP_TEMPLATE, &manifest).unwrap();

        assert!(!rendered.contains("use serde_json::Value"));
        assert!(!rendered.contains("fn schema_"));
    }

    #[test]
    fn invocation_template_uses_direct_http_runtime() {
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
        );

        let rendered = render_template("invoke.rs", INVOKE_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("pub async fn invoke_operation"));
        assert!(rendered.contains("struct DirectHttpInvoker"));
        assert!(rendered.contains("pub enum InvocationAdapterKind"));
        assert!(rendered.contains("pub enum DirectTypedInvocationStatus"));
        assert!(rendered.contains("pub struct InvocationAdapterContract"));
        assert!(rendered.contains("validate_invocation_adapter_contract_for_tool"));
        assert!(rendered.contains("requires_generated_cli_command: false"));
        assert!(rendered.contains("uses_temp_json_body_files: false"));
        assert!(rendered.contains("context.client.request(method, url)"));
        assert!(rendered.contains("crate::direct_http::build_request_parts"));
        assert!(rendered.contains("crate::direct_http::success_response"));
        assert!(rendered.contains("PathParam { wire_name: &'static str }"));
        assert!(rendered.contains("QueryParam { wire_name: &'static str }"));
        assert!(!rendered.contains("fn build_request_parts"));
        assert!(!rendered.contains("fn build_url"));
        assert!(!rendered.contains("fn collect_query_pairs"));
        assert!(!rendered.contains("fn parse_body_value"));
        assert!(!rendered.contains("write_json_body"));
        assert_eq!(manifest.mcp_runtime.invocation_adapter_kind, "direct_http");
        assert_eq!(
            manifest.mcp_runtime.invocation_adapter.kind_rust_variant,
            "InvocationAdapterKind::DirectHttp"
        );
        assert_eq!(
            manifest
                .mcp_runtime
                .invocation_adapter
                .direct_typed_invocation,
            "supported"
        );
        assert!(
            !manifest
                .mcp_runtime
                .invocation_adapter
                .requires_generated_cli_command
        );
        assert!(manifest
            .mcp_runtime
            .invocation_adapter_reason
            .contains("direct HTTP operation invocation"));
    }

    #[test]
    fn cargo_template_is_native_workspace_without_api_dependency() {
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            "https://example.test".to_string(),
            false,
            AuthKind::None,
        );

        let rendered = render_template("Cargo.toml", CARGO_TEMPLATE, &manifest).unwrap();

        assert!(!rendered.contains("members = [\"api\", \".\"]"));
        assert!(rendered.contains("name = \"petstore\""));
        assert!(!rendered.contains("petstore-api = { path = \"api\" }"));
    }
}
