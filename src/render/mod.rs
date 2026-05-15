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
use std::collections::BTreeSet;
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
    pub input_schema_literal: String,
    pub args: Vec<McpArg>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpArg {
    pub json_name: String,
    pub json_name_literal: String,
    pub cli_name: String,
    pub cli_name_literal: String,
    pub body_field: bool,
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

    pub fn with_openapi(mut self, api: &OpenAPI) -> Result<Self> {
        self.mcp_tools = mcp_tools(api, self.auth_env_var.as_deref())?;
        Ok(self)
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

const MCP_RESERVED_ARG_PREFIX: &str = "_pp_";

fn mcp_tools(api: &OpenAPI, auth_env_var: Option<&str>) -> Result<Vec<McpTool>> {
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
            api,
        )?;
        push_operation(
            &mut tools,
            "PUT",
            path,
            item.put.as_ref(),
            &path_params,
            auth_env_var,
            api,
        )?;
        push_operation(
            &mut tools,
            "POST",
            path,
            item.post.as_ref(),
            &path_params,
            auth_env_var,
            api,
        )?;
        push_operation(
            &mut tools,
            "DELETE",
            path,
            item.delete.as_ref(),
            &path_params,
            auth_env_var,
            api,
        )?;
        push_operation(
            &mut tools,
            "OPTIONS",
            path,
            item.options.as_ref(),
            &path_params,
            auth_env_var,
            api,
        )?;
        push_operation(
            &mut tools,
            "HEAD",
            path,
            item.head.as_ref(),
            &path_params,
            auth_env_var,
            api,
        )?;
        push_operation(
            &mut tools,
            "PATCH",
            path,
            item.patch.as_ref(),
            &path_params,
            auth_env_var,
            api,
        )?;
        push_operation(
            &mut tools,
            "TRACE",
            path,
            item.trace.as_ref(),
            &path_params,
            auth_env_var,
            api,
        )?;
    }
    Ok(tools)
}

fn push_operation(
    tools: &mut Vec<McpTool>,
    method: &str,
    path: &str,
    operation: Option<&Operation>,
    path_params: &[ReferenceOr<Parameter>],
    auth_env_var: Option<&str>,
    api: &OpenAPI,
) -> Result<()> {
    let Some(operation) = operation else {
        return Ok(());
    };
    let Some(raw_name) = operation.operation_id.clone() else {
        return Ok(());
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
        add_parameter(
            parameter,
            &mut properties,
            &mut required,
            &mut args,
            api,
            &name,
        )?;
    }
    add_body(
        operation.request_body.as_ref(),
        &mut properties,
        &mut required,
        &mut args,
        api,
        &name,
    )?;
    add_mcp_reserved_properties(&mut properties);

    let schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    });

    let input_schema = serde_json::to_string(&schema).expect("schema serializes");
    let input_schema_literal =
        serde_json::to_string(&input_schema).expect("schema literal serializes");
    tools.push(McpTool {
        name,
        description_literal: serde_json::to_string(&description).expect("description serializes"),
        description,
        input_schema,
        input_schema_literal,
        args,
    });
    Ok(())
}

fn add_mcp_reserved_properties(properties: &mut Map<String, Value>) {
    properties.insert(
        "_pp_fields".to_string(),
        json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "MCP-only response shaping: keep only these object dot paths."
        }),
    );
    properties.insert(
        "_pp_compact".to_string(),
        json!({
            "type": "boolean",
            "description": "MCP-only response shaping: remove nulls and empty arrays/objects from successful structured results."
        }),
    );
}

fn reject_reserved_arg(name: &str, tool_name: &str) -> Result<()> {
    if name.starts_with(MCP_RESERVED_ARG_PREFIX) {
        anyhow::bail!(
            "OpenAPI parameter '{name}' for MCP tool '{tool_name}' uses reserved pp namespace '{MCP_RESERVED_ARG_PREFIX}'"
        );
    }
    Ok(())
}

fn add_parameter(
    parameter: &ReferenceOr<Parameter>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    api: &OpenAPI,
    tool_name: &str,
) -> Result<()> {
    let ReferenceOr::Item(parameter) = parameter else {
        return Ok(());
    };
    let (data, is_path) = match parameter {
        Parameter::Query { parameter_data, .. } => (parameter_data, false),
        Parameter::Path { parameter_data, .. } => (parameter_data, true),
        _ => return Ok(()),
    };
    reject_reserved_arg(&data.name, tool_name)?;
    let schema = parameter_schema(data, api);
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
        body_field: false,
    });
    Ok(())
}

fn parameter_schema(data: &ParameterData, api: &OpenAPI) -> Value {
    match &data.format {
        ParameterSchemaOrContent::Schema(schema) => schema_json(schema, api),
        ParameterSchemaOrContent::Content(_) => json!({ "type": "string" }),
    }
}

fn add_body(
    request_body: Option<&ReferenceOr<RequestBody>>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    api: &OpenAPI,
    tool_name: &str,
) -> Result<()> {
    let Some(ReferenceOr::Item(body)) = request_body else {
        return Ok(());
    };
    let Some(media_type) = body
        .content
        .get("application/json")
        .or_else(|| body.content.values().next())
    else {
        return Ok(());
    };
    let Some(schema) = media_type.schema.as_ref() else {
        return Ok(());
    };
    let body_schema = schema_json(schema, api);
    if let Some(object) = body_schema.as_object() {
        if object.get("type").and_then(Value::as_str) == Some("object") {
            if let Some(Value::Object(body_properties)) = object.get("properties") {
                for (name, property_schema) in body_properties {
                    reject_reserved_arg(name, tool_name)?;
                    properties.insert(name.clone(), property_schema.clone());
                    let cli_name = name.to_kebab_case();
                    args.push(McpArg {
                        json_name_literal: serde_json::to_string(name)
                            .expect("arg name serializes"),
                        json_name: name.clone(),
                        cli_name_literal: serde_json::to_string(&cli_name)
                            .expect("arg name serializes"),
                        cli_name: "json-body".to_string(),
                        body_field: true,
                    });
                }
                if body.required {
                    if let Some(Value::Array(body_required)) = object.get("required") {
                        required.extend(
                            body_required
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string),
                        );
                    }
                }
                return Ok(());
            }
        }
    }
    reject_reserved_arg("body", tool_name)?;
    properties.insert("body".to_string(), body_schema);
    if body.required {
        required.push("body".to_string());
    }
    args.push(McpArg {
        json_name_literal: "\"body\"".to_string(),
        json_name: "body".to_string(),
        cli_name_literal: "\"json-body\"".to_string(),
        cli_name: "json-body".to_string(),
        body_field: false,
    });
    Ok(())
}

fn schema_json(schema: &ReferenceOr<Schema>, api: &OpenAPI) -> Value {
    schema_json_with_stack(schema, api, &mut BTreeSet::new())
}

fn schema_json_with_stack(
    schema: &ReferenceOr<Schema>,
    api: &OpenAPI,
    stack: &mut BTreeSet<String>,
) -> Value {
    match schema {
        ReferenceOr::Reference { reference } => resolve_schema_reference(reference, api, stack),
        ReferenceOr::Item(schema) => schema_kind_json(&schema.schema_kind, api, stack),
    }
}

fn boxed_schema_json_with_stack(
    schema: &ReferenceOr<Box<Schema>>,
    api: &OpenAPI,
    stack: &mut BTreeSet<String>,
) -> Value {
    match schema {
        ReferenceOr::Reference { reference } => resolve_schema_reference(reference, api, stack),
        ReferenceOr::Item(schema) => schema_kind_json(&schema.schema_kind, api, stack),
    }
}

fn resolve_schema_reference(reference: &str, api: &OpenAPI, stack: &mut BTreeSet<String>) -> Value {
    let Some(name) = reference.strip_prefix("#/components/schemas/") else {
        return json!({ "$ref": reference });
    };
    if !stack.insert(name.to_string()) {
        return json!({
            "type": "object",
            "description": format!("<recursive reference to {name}>")
        });
    }
    let value = api
        .components
        .as_ref()
        .and_then(|components| components.schemas.get(name))
        .map(|schema| schema_json_with_stack(schema, api, stack))
        .unwrap_or_else(|| json!({ "$ref": reference }));
    stack.remove(name);
    value
}

fn schema_kind_json(kind: &SchemaKind, api: &OpenAPI, stack: &mut BTreeSet<String>) -> Value {
    match kind {
        SchemaKind::Type(Type::String(_)) => json!({ "type": "string" }),
        SchemaKind::Type(Type::Number(_)) => json!({ "type": "number" }),
        SchemaKind::Type(Type::Integer(_)) => json!({ "type": "integer" }),
        SchemaKind::Type(Type::Boolean(_)) => json!({ "type": "boolean" }),
        SchemaKind::Type(Type::Array(array)) => {
            let mut value = json!({ "type": "array" });
            if let Some(items) = &array.items {
                value["items"] = boxed_schema_json_with_stack(items, api, stack);
            }
            value
        }
        SchemaKind::Type(Type::Object(object)) => {
            let mut properties = Map::new();
            for (name, schema) in &object.properties {
                properties.insert(
                    name.clone(),
                    boxed_schema_json_with_stack(schema, api, stack),
                );
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
    fn mcp_petstore_request_body_ref_is_flattened() {
        let spec = std::fs::read_to_string("testdata/petstore.yaml").unwrap();
        let api: OpenAPI = serde_yaml::from_str(&spec).unwrap();
        let tools = mcp_tools(&api, Some("SWAGGER_PETSTORE_API_KEY")).unwrap();
        let add_pet = tools.iter().find(|tool| tool.name == "add_pet").unwrap();
        let schema: Value = serde_json::from_str(&add_pet.input_schema).unwrap();
        let properties = schema["properties"].as_object().unwrap();

        assert!(properties.contains_key("name"));
        assert!(properties.contains_key("photoUrls"));
        assert!(properties.contains_key("tags"));
        assert_eq!(
            properties["tags"]["items"]["properties"]["name"]["type"],
            "string"
        );
        assert!(!serde_json::to_string(&schema).unwrap().contains("$ref"));
    }

    #[test]
    fn mcp_request_body_ref_cycle_uses_sentinel() {
        let spec = r##"
openapi: 3.0.3
info:
  title: Cycle API
  version: 1.0.0
paths:
  /cycles:
    post:
      operationId: createCycle
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/A'
      responses:
        '200':
          description: ok
components:
  schemas:
    A:
      type: object
      required: [b]
      properties:
        b:
          $ref: '#/components/schemas/B'
    B:
      type: object
      properties:
        a:
          $ref: '#/components/schemas/A'
"##;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let tools = mcp_tools(&api, None).unwrap();
        let tool = tools
            .iter()
            .find(|tool| tool.name == "create_cycle")
            .unwrap();
        let schema: Value = serde_json::from_str(&tool.input_schema).unwrap();

        assert_eq!(
            schema["properties"]["b"]["properties"]["a"]["type"],
            "object"
        );
        assert_eq!(
            schema["properties"]["b"]["properties"]["a"]["description"],
            "<recursive reference to A>"
        );
    }

    #[test]
    fn mcp_schema_includes_reserved_response_shaping_args() {
        let spec = std::fs::read_to_string("testdata/petstore.yaml").unwrap();
        let api: OpenAPI = serde_yaml::from_str(&spec).unwrap();
        let tools = mcp_tools(&api, None).unwrap();
        let add_pet = tools.iter().find(|tool| tool.name == "add_pet").unwrap();
        let schema: Value = serde_json::from_str(&add_pet.input_schema).unwrap();
        let properties = schema["properties"].as_object().unwrap();

        assert_eq!(properties["_pp_fields"]["type"], "array");
        assert_eq!(properties["_pp_compact"]["type"], "boolean");
        assert!(!add_pet
            .args
            .iter()
            .any(|arg| arg.json_name.starts_with("_pp_")));
    }

    #[test]
    fn mcp_reserved_query_parameter_is_generation_error() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Reserved API
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: _pp_fields
          in: query
          schema:
            type: string
      responses:
        '200':
          description: ok
"#;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err();

        assert!(error.to_string().contains("reserved pp namespace"));
        assert!(error.to_string().contains("_pp_fields"));
    }

    #[test]
    fn mcp_reserved_body_property_is_generation_error() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Reserved Body API
  version: "1.0.0"
paths:
  /items:
    post:
      operationId: createItem
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              properties:
                _pp_compact:
                  type: boolean
      responses:
        '200':
          description: ok
"#;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err();

        assert!(error.to_string().contains("reserved pp namespace"));
        assert!(error.to_string().contains("_pp_compact"));
    }

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
