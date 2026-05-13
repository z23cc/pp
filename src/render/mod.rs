//! Render the wrapper crate around progenitor's generated API crate.

use crate::spec::AuthKind;
use anyhow::{Context, Result};
use heck::{ToKebabCase, ToShoutySnakeCase, ToSnakeCase};
use minijinja::Environment;
use openapiv3::{
    OpenAPI, Operation, Parameter, ParameterData, ParameterSchemaOrContent, ReferenceOr,
    RequestBody, Schema, SchemaKind, Type,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

const CARGO_TEMPLATE: &str = include_str!("templates/Cargo.toml.j2");
const API_CARGO_TEMPLATE: &str = include_str!("templates/api_cargo.toml.j2");
const MAIN_TEMPLATE: &str = include_str!("templates/main.rs.j2");
const CLI_BUILDER_TEMPLATE: &str = include_str!("templates/cli_builder.rs.j2");
const CONTEXT_TEMPLATE: &str = include_str!("templates/context.rs.j2");
const AUTH_TEMPLATE: &str = include_str!("templates/auth.rs.j2");
const PRINT_TEMPLATE: &str = include_str!("templates/print.rs.j2");
const MCP_TEMPLATE: &str = include_str!("templates/mcp.rs.j2");

/// Facts required to render the generated wrapper crate.
#[derive(Debug, Clone, Serialize)]
pub struct WrapperManifest {
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
    pub mcp_tools: Vec<McpTool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub description_literal: String,
    pub input_schema: String,
    pub args: Vec<McpArg>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpArg {
    pub json_name: String,
    pub json_name_literal: String,
    pub cli_name: String,
    pub cli_name_literal: String,
}

impl WrapperManifest {
    /// Build template data from inspected facts and the selected progenitor crate name.
    pub fn new(
        bin_name: String,
        base_url: Option<String>,
        base_url_is_relative: bool,
        auth_kind: AuthKind,
        progenitor_lib_name: String,
    ) -> Self {
        let env_prefix = bin_name.to_shouty_snake_case();
        let progenitor_crate_name = progenitor_lib_name.replace('-', "_");
        Self {
            bin_name,
            base_url: base_url.unwrap_or_else(|| "http://localhost".to_string()),
            base_url_is_relative,
            auth_kind: auth_kind.clone(),
            progenitor_lib_name,
            progenitor_crate_name,
            token_env_var: format!("{env_prefix}_TOKEN"),
            api_key_env_var: format!("{env_prefix}_API_KEY"),
            basic_user_env_var: format!("{env_prefix}_USER"),
            basic_password_env_var: format!("{env_prefix}_PASSWORD"),
            auth_env_var: auth_env_var(&auth_kind, &env_prefix),
            mcp_tools: Vec::new(),
        }
    }

    pub fn with_openapi(mut self, api: &OpenAPI) -> Self {
        self.mcp_tools = mcp_tools(api, self.auth_env_var.as_deref());
        self
    }
}

/// Render all wrapper files into `out_dir`.
pub fn render(manifest: &WrapperManifest, out_dir: &Path) -> Result<()> {
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
        "print.rs",
        PRINT_TEMPLATE,
        manifest,
        &out_dir.join("src/print.rs"),
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

fn mcp_tools(api: &OpenAPI, auth_env_var: Option<&str>) -> Vec<McpTool> {
    let mut tools = Vec::new();
    for (path, item) in &api.paths.paths {
        let ReferenceOr::Item(item) = item else {
            continue;
        };
        let path_params = item.parameters.clone();
        push_operation(
            &mut tools,
            "GET",
            path,
            item.get.as_ref(),
            &path_params,
            auth_env_var,
        );
        push_operation(
            &mut tools,
            "PUT",
            path,
            item.put.as_ref(),
            &path_params,
            auth_env_var,
        );
        push_operation(
            &mut tools,
            "POST",
            path,
            item.post.as_ref(),
            &path_params,
            auth_env_var,
        );
        push_operation(
            &mut tools,
            "DELETE",
            path,
            item.delete.as_ref(),
            &path_params,
            auth_env_var,
        );
        push_operation(
            &mut tools,
            "OPTIONS",
            path,
            item.options.as_ref(),
            &path_params,
            auth_env_var,
        );
        push_operation(
            &mut tools,
            "HEAD",
            path,
            item.head.as_ref(),
            &path_params,
            auth_env_var,
        );
        push_operation(
            &mut tools,
            "PATCH",
            path,
            item.patch.as_ref(),
            &path_params,
            auth_env_var,
        );
        push_operation(
            &mut tools,
            "TRACE",
            path,
            item.trace.as_ref(),
            &path_params,
            auth_env_var,
        );
    }
    tools
}

fn push_operation(
    tools: &mut Vec<McpTool>,
    method: &str,
    path: &str,
    operation: Option<&Operation>,
    path_params: &[ReferenceOr<Parameter>],
    auth_env_var: Option<&str>,
) {
    let Some(operation) = operation else {
        return;
    };
    let Some(raw_name) = operation.operation_id.clone() else {
        return;
    };
    let name = raw_name.to_snake_case();
    let fallback_description = format!("{method} {path}");
    let mut description = operation
        .summary
        .as_deref()
        .or(operation.description.as_deref())
        .unwrap_or(&fallback_description)
        .chars()
        .take(1024)
        .collect::<String>();
    if let Some(auth_env_var) = auth_env_var {
        description.push_str(&format!(" [auth: {auth_env_var} env var]"));
    }

    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut args = Vec::new();

    for parameter in path_params.iter().chain(operation.parameters.iter()) {
        add_parameter(parameter, &mut properties, &mut required, &mut args);
    }
    add_body(
        operation.request_body.as_ref(),
        &mut properties,
        &mut required,
        &mut args,
    );

    let schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    });

    tools.push(McpTool {
        name,
        description_literal: serde_json::to_string(&description).expect("description serializes"),
        description,
        input_schema: serde_json::to_string(&schema).expect("schema serializes"),
        args,
    });
}

fn add_parameter(
    parameter: &ReferenceOr<Parameter>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
) {
    let ReferenceOr::Item(parameter) = parameter else {
        return;
    };
    let (data, is_path) = match parameter {
        Parameter::Query { parameter_data, .. } => (parameter_data, false),
        Parameter::Path { parameter_data, .. } => (parameter_data, true),
        _ => return,
    };
    let schema = parameter_schema(data);
    properties.insert(data.name.clone(), schema);
    if is_path || data.required {
        required.push(data.name.clone());
    }
    let cli_name = data.name.to_kebab_case();
    args.push(McpArg {
        json_name_literal: serde_json::to_string(&data.name).expect("arg name serializes"),
        json_name: data.name.clone(),
        cli_name_literal: serde_json::to_string(&cli_name).expect("arg name serializes"),
        cli_name,
    });
}

fn parameter_schema(data: &ParameterData) -> Value {
    match &data.format {
        ParameterSchemaOrContent::Schema(schema) => schema_json(schema),
        ParameterSchemaOrContent::Content(_) => json!({ "type": "string" }),
    }
}

fn add_body(
    request_body: Option<&ReferenceOr<RequestBody>>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
) {
    let Some(ReferenceOr::Item(body)) = request_body else {
        return;
    };
    let Some(media_type) = body
        .content
        .get("application/json")
        .or_else(|| body.content.values().next())
    else {
        return;
    };
    let Some(schema) = media_type.schema.as_ref() else {
        return;
    };
    if let ReferenceOr::Item(schema) = schema {
        if let SchemaKind::Type(Type::Object(object)) = &schema.schema_kind {
            for (name, property_schema) in &object.properties {
                properties.insert(name.clone(), boxed_schema_json(property_schema));
                let cli_name = name.to_kebab_case();
                args.push(McpArg {
                    json_name_literal: serde_json::to_string(name).expect("arg name serializes"),
                    json_name: name.clone(),
                    cli_name_literal: serde_json::to_string(&cli_name)
                        .expect("arg name serializes"),
                    cli_name,
                });
            }
            if body.required {
                required.extend(object.required.iter().cloned());
            }
            return;
        }
    }
    properties.insert("body".to_string(), schema_json(schema));
    if body.required {
        required.push("body".to_string());
    }
    args.push(McpArg {
        json_name_literal: "\"body\"".to_string(),
        json_name: "body".to_string(),
        cli_name_literal: "\"json-body\"".to_string(),
        cli_name: "json-body".to_string(),
    });
}

fn schema_json(schema: &ReferenceOr<Schema>) -> Value {
    match schema {
        ReferenceOr::Reference { reference } => json!({ "$ref": reference }),
        ReferenceOr::Item(schema) => schema_kind_json(&schema.schema_kind),
    }
}

fn boxed_schema_json(schema: &ReferenceOr<Box<Schema>>) -> Value {
    match schema {
        ReferenceOr::Reference { reference } => json!({ "$ref": reference }),
        ReferenceOr::Item(schema) => schema_kind_json(&schema.schema_kind),
    }
}

fn schema_kind_json(kind: &SchemaKind) -> Value {
    match kind {
        SchemaKind::Type(Type::String(_)) => json!({ "type": "string" }),
        SchemaKind::Type(Type::Number(_)) => json!({ "type": "number" }),
        SchemaKind::Type(Type::Integer(_)) => json!({ "type": "integer" }),
        SchemaKind::Type(Type::Boolean(_)) => json!({ "type": "boolean" }),
        SchemaKind::Type(Type::Array(_)) => json!({ "type": "array" }),
        SchemaKind::Type(Type::Object(object)) => {
            let mut properties = Map::new();
            for (name, schema) in &object.properties {
                properties.insert(name.clone(), boxed_schema_json(schema));
            }
            json!({ "type": "object", "properties": properties, "required": object.required })
        }
        _ => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_template_contains_workspace_and_api_dependency() {
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            Some("https://example.test".to_string()),
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
