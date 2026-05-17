//! Narrow API/MCP model derived from a normalized and sliced OpenAPI spec.
//!
//! This is intentionally not a replacement OpenAPI model. It only captures the
//! operation names, descriptions, arguments, input schemas, and reserved wrapper
//! inputs needed by the generated MCP wrapper.

mod arguments;
mod identity;
mod response;
mod schema;

use anyhow::Result;
use openapiv3::OpenAPI;
use serde::Serialize;

pub use arguments::{McpArg, McpArgBinding};
#[allow(unused_imports)]
pub use response::{
    McpDirectTypedInvocationStatus, McpInvocationAdapterContract, McpInvocationAdapterKind,
    McpResponseShaping, McpResponseShapingArg,
};

pub(crate) use identity::mcp_tools;
use response::mcp_response_shaping;

#[derive(Debug, Clone, Serialize)]
pub struct ApiModel {
    pub mcp_tools: Vec<McpTool>,
    pub mcp_response_shaping: McpResponseShaping,
    pub mcp_invocation_adapter: McpInvocationAdapterContract,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: String,
    pub args: Vec<McpArg>,
}

impl ApiModel {
    pub fn from_openapi(api: &OpenAPI, auth_env_var: Option<&str>) -> Result<Self> {
        Ok(Self {
            mcp_tools: mcp_tools(api, auth_env_var)?,
            mcp_response_shaping: mcp_response_shaping(),
            mcp_invocation_adapter: McpInvocationAdapterContract::progenitor_cli_bridge(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

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
        assert_eq!(
            model.mcp_invocation_adapter.kind,
            McpInvocationAdapterKind::ProgenitorCliBridge
        );
        assert_eq!(
            model.mcp_invocation_adapter.kind.as_str(),
            "progenitor_cli_bridge"
        );
        assert_eq!(
            model.mcp_invocation_adapter.direct_typed_invocation,
            McpDirectTypedInvocationStatus::Unsupported
        );
        assert!(model.mcp_invocation_adapter.requires_generated_cli_command);
        assert!(model
            .mcp_invocation_adapter
            .reason
            .contains("direct typed operation invocation is not supported"));
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
