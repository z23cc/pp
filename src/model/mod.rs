//! Narrow API/MCP model derived from a normalized and sliced OpenAPI spec.
//!
//! This is intentionally not a replacement OpenAPI model. It only captures the
//! operation names, descriptions, arguments, input schemas, and reserved wrapper
//! inputs needed by the generated MCP wrapper.

use crate::spec::traversal;
use anyhow::Result;
use heck::{ToKebabCase, ToSnakeCase};
use openapiv3::{
    OpenAPI, Parameter, ParameterData, ParameterSchemaOrContent, ReferenceOr, RequestBody, Schema,
    SchemaKind, Type,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const MCP_RESERVED_ARG_PREFIX: &str = "_pp_";
const MCP_FIELD_FILTER_ARG_NAME: &str = "_pp_fields";
const MCP_COMPACT_ARG_NAME: &str = "_pp_compact";

#[derive(Debug, Clone, Serialize)]
pub struct ApiModel {
    pub mcp_tools: Vec<McpTool>,
    pub mcp_response_shaping: McpResponseShaping,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: String,
    pub args: Vec<McpArg>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpArg {
    pub json_name: String,
    pub binding: McpArgBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpArgBinding {
    CliFlag { cli_name: String },
    FlattenedBodyField,
    WholeJsonBody,
}

impl McpArg {
    fn cli_flag(json_name: String, cli_name: String) -> Self {
        Self {
            json_name,
            binding: McpArgBinding::CliFlag { cli_name },
        }
    }

    fn flattened_body_field(json_name: String) -> Self {
        Self {
            json_name,
            binding: McpArgBinding::FlattenedBodyField,
        }
    }

    fn whole_json_body(json_name: String) -> Self {
        Self {
            json_name,
            binding: McpArgBinding::WholeJsonBody,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpResponseShaping {
    pub field_filter: McpResponseShapingArg,
    pub compact: McpResponseShapingArg,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpResponseShapingArg {
    pub json_name: String,
    pub schema: Value,
    pub invalid_type_message: String,
}

impl Default for McpResponseShaping {
    fn default() -> Self {
        mcp_response_shaping()
    }
}

impl ApiModel {
    pub fn from_openapi(api: &OpenAPI, auth_env_var: Option<&str>) -> Result<Self> {
        Ok(Self {
            mcp_tools: mcp_tools(api, auth_env_var)?,
            mcp_response_shaping: mcp_response_shaping(),
        })
    }
}

pub(crate) fn mcp_tools(api: &OpenAPI, auth_env_var: Option<&str>) -> Result<Vec<McpTool>> {
    let mut tools = Vec::new();
    let mut ctx = McpBuildContext {
        auth_env_var,
        api,
        seen_tool_names: BTreeMap::new(),
    };
    for operation in traversal::operations(api) {
        push_operation(&mut tools, operation, &mut ctx)?;
    }
    Ok(tools)
}

struct McpBuildContext<'a> {
    auth_env_var: Option<&'a str>,
    api: &'a OpenAPI,
    seen_tool_names: BTreeMap<String, String>,
}

fn push_operation(
    tools: &mut Vec<McpTool>,
    operation_ref: traversal::OperationRef<'_>,
    ctx: &mut McpBuildContext<'_>,
) -> Result<()> {
    let method = operation_ref.method_uppercase;
    let path = operation_ref.path;
    let path_params = operation_ref.path_parameters;
    let operation = operation_ref.operation;
    let Some(raw_name) = traversal::explicit_operation_id(operation).map(str::to_string) else {
        let derived_id = traversal::derived_operation_identifier(operation_ref.method, path);
        anyhow::bail!(
            "operation {method} {path} is missing operationId; explicit operationId is required for codegen/MCP identity. Add a stable operationId to this selected operation or exclude it from generation with `--exclude-operation \"{derived_id}\"`."
        );
    };
    let name = operation_name(&raw_name);
    if let Some(previous_operation_id) = ctx.seen_tool_names.insert(name.clone(), raw_name.clone())
    {
        anyhow::bail!(
            "MCP tool name collision: operationId '{previous_operation_id}' and operationId '{raw_name}' both produce MCP tool '{name}'"
        );
    }
    let fallback_description = format!("{method} {path}");
    let mut description = operation
        .summary
        .as_deref()
        .or(operation.description.as_deref())
        .unwrap_or(&fallback_description)
        .chars()
        .take(1024)
        .collect::<String>();
    if let Some(auth_env_var) = ctx.auth_env_var {
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
            ctx.api,
            &name,
            &raw_name,
        )?;
    }
    add_body(
        operation.request_body.as_ref(),
        &mut properties,
        &mut required,
        &mut args,
        ctx.api,
        &name,
        &raw_name,
    )?;
    add_mcp_reserved_properties(&mut properties);

    let schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    });

    let input_schema = serde_json::to_string(&schema).expect("schema serializes");
    tools.push(McpTool {
        name,
        description,
        input_schema,
        args,
    });
    Ok(())
}

fn operation_name(operation_id: &str) -> String {
    operation_id.to_snake_case()
}

fn mcp_response_shaping() -> McpResponseShaping {
    let field_filter_message = "_pp_fields must be an array of dot paths".to_string();
    let compact_message = "_pp_compact must be a boolean".to_string();
    McpResponseShaping {
        field_filter: McpResponseShapingArg {
            json_name: MCP_FIELD_FILTER_ARG_NAME.to_string(),
            schema: json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "MCP-only response shaping: keep only these object dot paths."
            }),
            invalid_type_message: field_filter_message,
        },
        compact: McpResponseShapingArg {
            json_name: MCP_COMPACT_ARG_NAME.to_string(),
            schema: json!({
                "type": "boolean",
                "description": "MCP-only response shaping: remove nulls and empty arrays/objects from successful structured results."
            }),
            invalid_type_message: compact_message,
        },
    }
}

fn add_mcp_reserved_properties(properties: &mut Map<String, Value>) {
    let shaping = mcp_response_shaping();
    properties.insert(shaping.field_filter.json_name, shaping.field_filter.schema);
    properties.insert(shaping.compact.json_name, shaping.compact.schema);
}

fn reject_reserved_arg(name: &str, tool_name: &str, operation_id: &str) -> Result<()> {
    if name.starts_with(MCP_RESERVED_ARG_PREFIX) {
        anyhow::bail!(
            "OpenAPI argument '{name}' for MCP tool '{tool_name}' (operationId '{operation_id}') uses reserved pp namespace '{MCP_RESERVED_ARG_PREFIX}'"
        );
    }
    Ok(())
}

fn reject_duplicate_arg(
    properties: &Map<String, Value>,
    name: &str,
    tool_name: &str,
    operation_id: &str,
    source: &str,
) -> Result<()> {
    if properties.contains_key(name) {
        anyhow::bail!(
            "MCP argument collision for tool '{tool_name}' (operationId '{operation_id}'): argument '{name}' from {source} duplicates an existing MCP argument"
        );
    }
    Ok(())
}

fn reject_cli_arg_collision(
    args: &[McpArg],
    cli_name: &str,
    json_name: &str,
    tool_name: &str,
    operation_id: &str,
    source: &str,
) -> Result<()> {
    if cli_name == "json-body" {
        anyhow::bail!(
            "MCP CLI argument collision for tool '{tool_name}' (operationId '{operation_id}'): argument '{json_name}' from {source} maps to reserved generated flag '--json-body'"
        );
    }
    if let Some(existing) = args.iter().find(|arg| {
        matches!(
            &arg.binding,
            McpArgBinding::CliFlag { cli_name: existing_cli_name } if existing_cli_name == cli_name
        )
    }) {
        anyhow::bail!(
            "MCP CLI argument collision for tool '{tool_name}' (operationId '{operation_id}'): argument '{json_name}' from {source} maps to '--{cli_name}', already used by argument '{}'",
            existing.json_name
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
    operation_id: &str,
) -> Result<()> {
    let ReferenceOr::Item(parameter) = parameter else {
        return Ok(());
    };
    let (data, is_path) = match parameter {
        Parameter::Query { parameter_data, .. } => (parameter_data, false),
        Parameter::Path { parameter_data, .. } => (parameter_data, true),
        _ => return Ok(()),
    };
    reject_reserved_arg(&data.name, tool_name, operation_id)?;
    reject_duplicate_arg(
        properties,
        &data.name,
        tool_name,
        operation_id,
        "OpenAPI parameter",
    )?;
    let cli_name = data.name.to_kebab_case();
    reject_cli_arg_collision(
        args,
        &cli_name,
        &data.name,
        tool_name,
        operation_id,
        "OpenAPI parameter",
    )?;
    let schema = parameter_schema(data, api);
    properties.insert(data.name.clone(), schema);
    if is_path || data.required {
        required.push(data.name.clone());
    }
    args.push(McpArg::cli_flag(data.name.clone(), cli_name));
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
    operation_id: &str,
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
                let has_flattening_collision = body_properties
                    .keys()
                    .any(|name| properties.contains_key(name));
                if has_flattening_collision {
                    return add_synthetic_body_arg(
                        body_schema,
                        body.required,
                        properties,
                        required,
                        args,
                        tool_name,
                        operation_id,
                    );
                }

                for (name, property_schema) in body_properties {
                    reject_reserved_arg(name, tool_name, operation_id)?;
                    properties.insert(name.clone(), property_schema.clone());
                    args.push(McpArg::flattened_body_field(name.clone()));
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
    add_synthetic_body_arg(
        body_schema,
        body.required,
        properties,
        required,
        args,
        tool_name,
        operation_id,
    )
}

fn add_synthetic_body_arg(
    body_schema: Value,
    body_required: bool,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    tool_name: &str,
    operation_id: &str,
) -> Result<()> {
    reject_reserved_arg("body", tool_name, operation_id)?;
    reject_duplicate_arg(
        properties,
        "body",
        tool_name,
        operation_id,
        "synthetic request body argument",
    )?;
    properties.insert("body".to_string(), body_schema);
    if body_required {
        required.push("body".to_string());
    }
    args.push(McpArg::whole_json_body("body".to_string()));
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

    fn has_cli_arg(tool: &McpTool, json_name: &str, cli_name: &str) -> bool {
        tool.args.iter().any(|arg| {
            arg.json_name == json_name
                && matches!(
                    &arg.binding,
                    McpArgBinding::CliFlag { cli_name: actual_cli_name } if actual_cli_name == cli_name
                )
        })
    }

    fn has_flattened_body_field(tool: &McpTool, json_name: &str) -> bool {
        tool.args.iter().any(|arg| {
            arg.json_name == json_name && matches!(arg.binding, McpArgBinding::FlattenedBodyField)
        })
    }

    fn has_whole_json_body(tool: &McpTool, json_name: &str) -> bool {
        tool.args.iter().any(|arg| {
            arg.json_name == json_name && matches!(arg.binding, McpArgBinding::WholeJsonBody)
        })
    }

    #[test]
    fn api_model_exposes_response_shaping_runtime_inputs() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Shape Metadata API
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      responses:
        '200':
          description: ok
"#;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let model = ApiModel::from_openapi(&api, None).unwrap();

        assert_eq!(
            model.mcp_response_shaping.field_filter.json_name,
            "_pp_fields"
        );
        assert_eq!(
            model.mcp_response_shaping.field_filter.schema["items"]["type"],
            "string"
        );
        assert_eq!(
            model.mcp_response_shaping.field_filter.invalid_type_message,
            "_pp_fields must be an array of dot paths"
        );
        assert_eq!(model.mcp_response_shaping.compact.json_name, "_pp_compact");
        assert_eq!(model.mcp_response_shaping.compact.schema["type"], "boolean");
    }

    #[test]
    fn mcp_tools_assign_stable_semantic_names() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Runtime Metadata API
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      responses:
        '200':
          description: ok
  /items/{id}:
    get:
      operationId: getItem
      responses:
        '200':
          description: ok
"#;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let tools = mcp_tools(&api, None).unwrap();

        assert_eq!(tools[0].name, "list_items");
        assert_eq!(tools[1].name, "get_item");
        assert_eq!(tools[0].description, "GET /items");
        assert_eq!(tools[1].description, "GET /items/{id}");
    }

    #[test]
    fn mcp_petstore_request_body_ref_is_flattened() {
        let spec = std::fs::read_to_string("testdata/petstore.yaml").unwrap();
        let mut api: OpenAPI = serde_yaml::from_str(&spec).unwrap();
        api.paths.paths.retain(|path, _| !path.starts_with("/user"));
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
        assert!(has_flattened_body_field(add_pet, "name"));
        assert!(has_flattened_body_field(add_pet, "photoUrls"));
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
        let mut api: OpenAPI = serde_yaml::from_str(&spec).unwrap();
        api.paths.paths.retain(|path, _| !path.starts_with("/user"));
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
    fn mcp_includes_path_level_parameters() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Path Parameters API
  version: "1.0.0"
paths:
  /items/{id}:
    parameters:
      - name: id
        in: path
        required: true
        schema:
          type: string
    get:
      operationId: getItem
      responses:
        '200':
          description: ok
"#;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let tools = mcp_tools(&api, None).unwrap();
        let tool = tools.iter().find(|tool| tool.name == "get_item").unwrap();
        let schema: Value = serde_json::from_str(&tool.input_schema).unwrap();

        assert_eq!(schema["properties"]["id"]["type"], "string");
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("id")));
        assert!(has_cli_arg(tool, "id", "id"));
    }

    #[test]
    fn mcp_missing_operation_id_is_generation_error() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Missing Operation ID API
  version: "1.0.0"
paths:
  /items/{id}:
    patch:
      responses:
        '200':
          description: ok
"#;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err().to_string();

        assert!(error.contains("operation PATCH /items/{id} is missing operationId"));
        assert!(error.contains("explicit operationId is required for codegen/MCP identity"));
        assert!(error.contains("--exclude-operation \"patch /items/{id}\""));
    }

    #[test]
    fn mcp_blank_operation_id_is_generation_error() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Blank Operation ID API
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: "   "
      responses:
        '200':
          description: ok
"#;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err().to_string();

        assert!(error.contains("operation GET /items is missing operationId"));
        assert!(error.contains("explicit operationId is required for codegen/MCP identity"));
    }

    #[test]
    fn mcp_operation_id_snake_case_collision_is_generation_error() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Collision API
  version: "1.0.0"
paths:
  /first:
    get:
      operationId: get-user
      responses:
        '200':
          description: ok
  /second:
    get:
      operationId: get_user
      responses:
        '200':
          description: ok
"#;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err().to_string();

        assert!(error.contains("MCP tool name collision"));
        assert!(error.contains("get-user"));
        assert!(error.contains("get_user"));
        assert!(error.contains("MCP tool 'get_user'"));
    }

    #[test]
    fn mcp_flattened_body_property_collision_falls_back_to_whole_body_arg() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Body Collision API
  version: "1.0.0"
paths:
  /items:
    post:
      operationId: createItem
      parameters:
        - name: id
          in: query
          schema:
            type: string
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              properties:
                id:
                  type: string
      responses:
        '200':
          description: ok
"#;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let tools = mcp_tools(&api, None).unwrap();
        let tool = tools
            .iter()
            .find(|tool| tool.name == "create_item")
            .unwrap();
        let schema: Value = serde_json::from_str(&tool.input_schema).unwrap();
        let properties = schema["properties"].as_object().unwrap();

        assert!(properties.contains_key("id"));
        assert!(properties.contains_key("body"));
        assert!(has_cli_arg(tool, "id", "id"));
        assert!(has_whole_json_body(tool, "body"));
        assert!(!has_flattened_body_field(tool, "id"));
    }

    #[test]
    fn mcp_synthetic_body_arg_cannot_duplicate_parameter_named_body() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Synthetic Body Collision API
  version: "1.0.0"
paths:
  /items:
    post:
      operationId: replaceItems
      parameters:
        - name: body
          in: query
          schema:
            type: string
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
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err().to_string();

        assert!(error.contains("MCP argument collision"));
        assert!(error.contains("replace_items"));
        assert!(error.contains("replaceItems"));
        assert!(error.contains("body"));
        assert!(error.contains("synthetic request body argument"));
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
    fn mcp_query_parameter_cannot_map_to_reserved_json_body_flag() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Json Body Flag Collision API
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: json_body
          in: query
          schema:
            type: string
      responses:
        '200':
          description: ok
"#;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err().to_string();

        assert!(error.contains("MCP CLI argument collision"));
        assert!(error.contains("json_body"));
        assert!(error.contains("--json-body"));
        assert!(error.contains("reserved generated flag"));
    }

    #[test]
    fn mcp_query_parameters_cannot_share_generated_cli_flag() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Cli Flag Collision API
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: foo_bar
          in: query
          schema:
            type: string
        - name: foo-bar
          in: query
          schema:
            type: string
      responses:
        '200':
          description: ok
"#;
        let api: OpenAPI = serde_yaml::from_str(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err().to_string();

        assert!(error.contains("MCP CLI argument collision"));
        assert!(error.contains("foo-bar"));
        assert!(error.contains("foo_bar"));
        assert!(error.contains("--foo-bar"));
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
}
