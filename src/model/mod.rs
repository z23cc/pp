//! Narrow API/MCP model derived from a parsed and optionally sliced OpenAPI spec.
//!
//! This is intentionally not a replacement OpenAPI model. It only captures the
//! operation names, descriptions, arguments, input schemas, and reserved wrapper
//! inputs needed by the generated MCP wrapper.

mod arguments;
mod diagnostics;
mod direct_http_plan;
mod identity;
mod response;
mod value_kind;

use crate::backend::{BackendCapabilities, DirectInvocationRequirements};
use crate::spec::PpSpec;
use anyhow::Result;
use serde::Serialize;

pub use arguments::{GeneratedArg, McpArg, McpArgBinding};
#[allow(unused_imports)]
pub use response::{
    McpDirectTypedInvocationStatus, McpInvocationAdapterContract, McpInvocationAdapterKind,
    McpResponseShaping, McpResponseShapingArg,
};
pub use value_kind::{ArgValueKind, PrimitiveKind};

pub(crate) use identity::mcp_model;
use response::mcp_response_shaping;

#[derive(Debug, Clone, Serialize)]
pub struct ApiModel {
    /// Canonical generated operation table used by MCP today and by the native CLI renderer later.
    pub mcp_tools: Vec<McpTool>,
    pub unsupported_mcp_operations: Vec<McpUnsupportedOperation>,
    pub mcp_response_shaping: McpResponseShaping,
    pub mcp_invocation_adapter: McpInvocationAdapterContract,
}

pub type McpTool = GeneratedOperation;

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedOperation {
    pub name: String,
    pub description: String,
    pub input_schema: String,
    pub method: String,
    pub path_template: String,
    pub args: Vec<GeneratedArg>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpUnsupportedOperation {
    pub operation_id: Option<String>,
    pub method: String,
    pub path: String,
    pub reason: String,
}

#[cfg(test)]
fn mcp_tools(api: &PpSpec, auth_env_var: Option<&str>) -> Result<Vec<McpTool>> {
    Ok(mcp_model_for_tests(api, auth_env_var)?.tools)
}

#[cfg(test)]
fn mcp_model_for_tests(api: &PpSpec, auth_env_var: Option<&str>) -> Result<identity::McpModel> {
    let capabilities = BackendCapabilities::native_http();
    mcp_model(api, auth_env_var, &capabilities.direct_invocation)
}

impl ApiModel {
    #[allow(dead_code)]
    pub fn from_spec(api: &PpSpec, auth_env_var: Option<&str>) -> Result<Self> {
        let capabilities = BackendCapabilities::native_http();
        Self::from_spec_with_direct_invocation(api, auth_env_var, &capabilities.direct_invocation)
    }

    pub(crate) fn from_spec_with_direct_invocation(
        api: &PpSpec,
        auth_env_var: Option<&str>,
        capabilities: &DirectInvocationRequirements,
    ) -> Result<Self> {
        let mcp_model = mcp_model(api, auth_env_var, capabilities)?;
        Ok(Self {
            mcp_tools: mcp_model.tools,
            unsupported_mcp_operations: mcp_model.unsupported_operations,
            mcp_response_shaping: mcp_response_shaping(),
            mcp_invocation_adapter: McpInvocationAdapterContract::direct_http(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn has_path_param(tool: &McpTool, json_name: &str, wire_name: &str) -> bool {
        tool.args.iter().any(|arg| {
            arg.json_name == json_name
                && matches!(
                    &arg.binding,
                    McpArgBinding::PathParam { wire_name: actual_wire_name } if actual_wire_name == wire_name
                )
        })
    }

    fn has_flattened_body_field(tool: &McpTool, json_name: &str) -> bool {
        tool.args.iter().any(|arg| {
            arg.json_name == json_name && matches!(arg.binding, McpArgBinding::FlattenedBodyField)
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let model = ApiModel::from_spec(&api, None).unwrap();

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
            McpInvocationAdapterKind::DirectHttp
        );
        assert_eq!(model.mcp_invocation_adapter.kind.as_str(), "direct_http");
        assert_eq!(
            model.mcp_invocation_adapter.direct_typed_invocation,
            McpDirectTypedInvocationStatus::Supported
        );
        assert!(!model.mcp_invocation_adapter.requires_generated_cli_command);
        assert!(model
            .mcp_invocation_adapter
            .reason
            .contains("direct HTTP operation invocation"));
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let tools = mcp_tools(&api, None).unwrap();

        assert_eq!(tools[0].name, "list_items");
        assert_eq!(tools[1].name, "get_item");
        assert_eq!(tools[0].description, "GET /items");
        assert_eq!(tools[1].description, "GET /items/{id}");
        assert_eq!(tools[0].method, "GET");
        assert_eq!(tools[0].path_template, "/items");
        assert_eq!(tools[1].method, "GET");
        assert_eq!(tools[1].path_template, "/items/{id}");
    }

    #[test]
    fn mcp_petstore_request_body_ref_is_flattened() {
        let spec = std::fs::read_to_string("testdata/petstore.yaml").unwrap();
        let mut api = crate::spec::parse_spec_for_tests(&spec).unwrap();
        api.retain_paths_for_tests(|path| !path.starts_with("/user"));
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
        let name_arg = add_pet
            .args
            .iter()
            .find(|arg| arg.json_name == "name")
            .expect("name arg");
        assert!(name_arg.required);
        assert_eq!(name_arg.value_kind, ArgValueKind::String);
        let photo_urls_arg = add_pet
            .args
            .iter()
            .find(|arg| arg.json_name == "photoUrls")
            .expect("photoUrls arg");
        assert!(photo_urls_arg.required);
        assert_eq!(
            photo_urls_arg.value_kind,
            ArgValueKind::PrimitiveArray {
                item: PrimitiveKind::String
            }
        );
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
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
        let mut api = crate::spec::parse_spec_for_tests(&spec).unwrap();
        api.retain_paths_for_tests(|path| !path.starts_with("/user"));
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let tools = mcp_tools(&api, None).unwrap();
        let tool = tools.iter().find(|tool| tool.name == "get_item").unwrap();
        let schema: Value = serde_json::from_str(&tool.input_schema).unwrap();

        assert_eq!(schema["properties"]["id"]["type"], "string");
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("id")));
        assert!(has_path_param(tool, "id", "id"));
        let id_arg = tool
            .args
            .iter()
            .find(|arg| arg.json_name == "id")
            .expect("id arg");
        assert!(id_arg.required);
        assert_eq!(id_arg.value_kind, ArgValueKind::String);
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err().to_string();

        assert!(error.contains("MCP tool name collision"));
        assert!(error.contains("get-user"));
        assert!(error.contains("get_user"));
        assert!(error.contains("MCP tool 'get_user'"));
    }

    #[test]
    fn unsupported_operation_name_does_not_poison_later_supported_collision() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Unsupported Collision API
  version: "1.0.0"
paths:
  /unsupported:
    get:
      operationId: search-items
      parameters:
        - name: filter
          in: query
          schema:
            type: object
            properties:
              status:
                type: string
      responses:
        '200':
          description: ok
  /supported:
    get:
      operationId: search_items
      responses:
        '200':
          description: ok
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let model = mcp_model_for_tests(&api, None).unwrap();

        assert_eq!(model.tools.len(), 1);
        assert_eq!(model.tools[0].name, "search_items");
        assert_eq!(model.tools[0].path_template, "/supported");
        assert_eq!(model.unsupported_operations.len(), 1);
        assert_eq!(
            model.unsupported_operations[0].operation_id.as_deref(),
            Some("search-items")
        );
    }

    #[test]
    fn unsupported_parameter_shapes_are_excluded_from_direct_invocation() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Unsupported Parameter Shapes API
  version: "1.0.0"
paths:
  /query-object:
    get:
      operationId: queryObject
      parameters:
        - name: filter
          in: query
          schema:
            type: object
            properties:
              status:
                type: string
      responses:
        '200':
          description: ok
  /query-composed:
    get:
      operationId: queryComposed
      parameters:
        - name: filter
          in: query
          schema:
            allOf:
              - type: string
      responses:
        '200':
          description: ok
  /query-missing-type:
    get:
      operationId: queryMissingType
      parameters:
        - name: filter
          in: query
          schema: {}
      responses:
        '200':
          description: ok
  /path-object/{id}:
    get:
      operationId: pathObject
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: object
            properties:
              value:
                type: string
      responses:
        '200':
          description: ok
  /path-composed/{id}:
    get:
      operationId: pathComposed
      parameters:
        - name: id
          in: path
          required: true
          schema:
            allOf:
              - type: string
      responses:
        '200':
          description: ok
  /path-missing-type/{id}:
    get:
      operationId: pathMissingType
      parameters:
        - name: id
          in: path
          required: true
          schema: {}
      responses:
        '200':
          description: ok
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let model = mcp_model_for_tests(&api, None).unwrap();

        assert!(model.tools.is_empty());
        assert_eq!(model.unsupported_operations.len(), 6);
        for unsupported in &model.unsupported_operations {
            assert!(unsupported
                .reason
                .starts_with(arguments::DIRECT_UNSUPPORTED_PREFIX));
        }
        assert!(model
            .unsupported_operations
            .iter()
            .any(|operation| operation.reason.contains("object parameter 'filter'")));
        assert!(model
            .unsupported_operations
            .iter()
            .any(|operation| operation.reason.contains("without primitive schema type")));
    }

    #[test]
    fn unsupported_query_array_serialization_is_excluded() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Unsupported Query Array Serialization API
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: tags
          in: query
          explode: false
          schema:
            type: array
            items:
              type: string
      responses:
        '200':
          description: ok
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let capabilities = BackendCapabilities::native_http();
        let model = mcp_model(&api, None, &capabilities.direct_invocation).unwrap();

        assert!(model.tools.is_empty());
        assert_eq!(model.unsupported_operations.len(), 1);
        assert!(model.unsupported_operations[0]
            .reason
            .contains("non-exploded query array parameter 'tags'"));
    }

    #[test]
    fn direct_invocation_capabilities_gate_query_array_support() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Capability Gated Query Array API
  version: "1.0.0"
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let mut query_arrays_disabled = BackendCapabilities::native_http();
        query_arrays_disabled
            .direct_invocation
            .parameters
            .supports_query_arrays = false;

        let disabled_model =
            mcp_model(&api, None, &query_arrays_disabled.direct_invocation).unwrap();

        assert!(disabled_model.tools.is_empty());
        assert_eq!(disabled_model.unsupported_operations.len(), 1);
        assert!(disabled_model.unsupported_operations[0]
            .reason
            .contains("array parameter 'tags'"));

        let native_capabilities = BackendCapabilities::native_http();
        let native_model = mcp_model(&api, None, &native_capabilities.direct_invocation).unwrap();

        assert_eq!(native_model.tools.len(), 1);
        assert!(native_model.unsupported_operations.is_empty());
        let arg = native_model.tools[0]
            .args
            .iter()
            .find(|arg| arg.json_name == "tags")
            .expect("tags arg");
        assert!(!arg.required);
        assert_eq!(
            arg.value_kind,
            ArgValueKind::PrimitiveArray {
                item: PrimitiveKind::String
            }
        );
    }

    #[test]
    fn mcp_flattened_body_property_collision_is_unsupported() {
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let model = mcp_model_for_tests(&api, None).unwrap();

        assert!(model.tools.is_empty());
        assert_eq!(model.unsupported_operations.len(), 1);
        assert!(model.unsupported_operations[0]
            .reason
            .contains("flattened JSON request body field collision"));
    }

    #[test]
    fn mcp_empty_request_body_content_is_unsupported() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Empty Body API
  version: "1.0.0"
paths:
  /items:
    post:
      operationId: createItem
      requestBody:
        required: true
        content: {}
      responses:
        '200':
          description: ok
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let model = mcp_model_for_tests(&api, None).unwrap();

        assert!(model.tools.is_empty());
        assert_eq!(model.unsupported_operations.len(), 1);
        assert!(model.unsupported_operations[0]
            .reason
            .contains("requestBody without JSON content"));
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err().to_string();

        assert!(error.contains("MCP argument collision"));
        assert!(error.contains("replace_items"));
        assert!(error.contains("replaceItems"));
        assert!(error.contains("body"));
        assert!(error.contains("synthetic request body argument"));
    }

    #[test]
    fn mcp_reserved_operation_name_is_generation_error() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Reserved Operation API
  version: "1.0.0"
paths:
  /events:
    get:
      operationId: mcp
      responses:
        '200':
          description: ok
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err().to_string();

        assert!(error.contains("reserved generated CLI command"));
        assert!(error.contains("mcp"));
    }

    #[test]
    fn mcp_reserved_cli_arg_name_is_generation_error() {
        let spec = r#"
openapi: 3.0.0
info:
  title: Reserved CLI Arg API
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: json
          in: query
          schema:
            type: string
      responses:
        '200':
          description: ok
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err().to_string();

        assert!(error.contains("reserved by the generated CLI"));
        assert!(error.contains("json"));
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
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
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let error = mcp_tools(&api, None).unwrap_err();

        assert!(error.to_string().contains("reserved pp namespace"));
        assert!(error.to_string().contains("_pp_compact"));
    }

    #[test]
    fn openapi_31_subset_projects_nullable_defs_query_arrays_and_body() {
        let spec = r##"
openapi: 3.1.0
info: { title: OpenAPI 31 Subset API, version: '1.0' }
paths:
  /items/{id}:
    post:
      operationId: updateItem
      parameters:
        - name: id
          in: path
          required: true
          schema: { type: string }
        - name: tags
          in: query
          explode: true
          schema:
            type: array
            items: { type: string }
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/UpdateItem'
      responses:
        '200': { description: ok }
components:
  schemas:
    UpdateItem:
      type: object
      required: [name]
      properties:
        name:
          type: [string, 'null']
        rating:
          $ref: '#/$defs/RatingAlias'
      $defs:
        RatingAlias:
          $ref: '#/$defs/Rating'
        Rating:
          type: integer
"##;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let tools = mcp_tools(&api, None).unwrap();
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        let schema: Value = serde_json::from_str(&tool.input_schema).unwrap();

        assert_eq!(schema["properties"]["id"]["type"], "string");
        assert_eq!(schema["properties"]["tags"]["type"], "array");
        assert_eq!(schema["properties"]["tags"]["items"]["type"], "string");
        assert_eq!(
            schema["properties"]["name"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(schema["properties"]["rating"]["type"], "integer");
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("name")));
        assert!(tool.args.iter().any(|arg| arg.json_name == "name"
            && matches!(
                arg.value_kind,
                ArgValueKind::NullablePrimitive {
                    item: PrimitiveKind::String
                }
            )));
        assert!(tool.args.iter().any(|arg| arg.json_name == "tags"
            && matches!(
                arg.value_kind,
                ArgValueKind::PrimitiveArray {
                    item: PrimitiveKind::String
                }
            )));
    }

    #[test]
    fn openapi_31_unsupported_json_schema_feature_is_operation_audit() {
        let spec = r#"
openapi: 3.1.0
info: { title: Unsupported 31 API, version: '1.0' }
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: filter
          in: query
          schema:
            oneOf:
              - type: string
              - type: integer
      responses:
        '200': { description: ok }
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let model = mcp_model_for_tests(&api, None).unwrap();
        assert!(model.tools.is_empty());
        assert_eq!(model.unsupported_operations.len(), 1);
        assert!(model.unsupported_operations[0]
            .reason
            .contains("unsupported JSON Schema feature 'oneOf'"));
        assert!(model.unsupported_operations[0]
            .reason
            .contains("parameter 'filter'"));
    }

    #[test]
    fn openapi_31_nested_unsupported_json_schema_feature_is_operation_audit() {
        let spec = r#"
openapi: 3.1.0
info: { title: Nested Unsupported 31 API, version: '1.0' }
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
                filter:
                  type: object
                  properties:
                    mode:
                      oneOf:
                        - type: string
                        - type: integer
      responses:
        '200': { description: ok }
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let model = mcp_model_for_tests(&api, None).unwrap();
        assert!(model.tools.is_empty());
        assert!(model.unsupported_operations[0]
            .reason
            .contains("unsupported JSON request body field 'filter'"));
        assert!(model.unsupported_operations[0]
            .reason
            .contains("unsupported JSON Schema feature 'oneOf'"));
    }

    #[test]
    fn malformed_parameters_are_unsupported_instead_of_defaulted() {
        let missing_name = r#"
openapi: 3.1.0
info: { title: Missing Parameter Name API, version: '1.0' }
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - in: query
          schema: { type: string }
      responses:
        '200': { description: ok }
"#;
        let api = crate::spec::parse_spec_for_tests(missing_name).unwrap();
        let model = mcp_model_for_tests(&api, None).unwrap();
        assert!(model.tools.is_empty());
        assert!(model.unsupported_operations[0]
            .reason
            .contains("parameter without non-empty string name"));

        let missing_location = r#"
openapi: 3.1.0
info: { title: Missing Parameter Location API, version: '1.0' }
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: q
          schema: { type: string }
      responses:
        '200': { description: ok }
"#;
        let api = crate::spec::parse_spec_for_tests(missing_location).unwrap();
        let model = mcp_model_for_tests(&api, None).unwrap();
        assert!(model.tools.is_empty());
        assert!(model.unsupported_operations[0]
            .reason
            .contains("parameter 'q' without supported 'in' location"));
    }

    #[test]
    fn free_form_object_request_body_is_unsupported_instead_of_dropped() {
        let spec = r#"
openapi: 3.1.0
info: { title: Free Form Body API, version: '1.0' }
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
              additionalProperties: true
      responses:
        '200': { description: ok }
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let model = mcp_model_for_tests(&api, None).unwrap();
        assert!(model.tools.is_empty());
        assert!(model.unsupported_operations[0]
            .reason
            .contains("unsupported JSON request body schema"));
        assert!(model.unsupported_operations[0]
            .reason
            .contains("unsupported JSON Schema feature 'additionalProperties'"));
    }

    #[test]
    fn openapi_31_required_nullable_parameter_is_unsupported() {
        let spec = r#"
openapi: 3.1.0
info: { title: Nullable Param API, version: '1.0' }
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: q
          in: query
          required: true
          schema:
            type: [string, 'null']
      responses:
        '200': { description: ok }
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let model = mcp_model_for_tests(&api, None).unwrap();
        assert!(model.tools.is_empty());
        assert!(model.unsupported_operations[0]
            .reason
            .contains("required nullable parameter 'q'"));
    }

    #[test]
    fn openapi_31_nullable_query_array_items_are_unsupported() {
        let spec = r#"
openapi: 3.1.0
info: { title: Nullable Array Items API, version: '1.0' }
paths:
  /items:
    get:
      operationId: listItems
      parameters:
        - name: tags
          in: query
          explode: true
          schema:
            type: array
            items:
              type: [string, 'null']
      responses:
        '200': { description: ok }
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let model = mcp_model_for_tests(&api, None).unwrap();
        assert!(model.tools.is_empty());
        assert_eq!(model.unsupported_operations.len(), 1);
        assert!(model.unsupported_operations[0]
            .reason
            .contains("nullable array items for parameter 'tags'"));
    }

    #[test]
    fn nullable_root_object_request_body_uses_whole_json_body_arg() {
        let spec = r#"
openapi: 3.1.0
info: { title: Nullable Root Body API, version: '1.0' }
paths:
  /items:
    post:
      operationId: createItem
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: [object, 'null']
              required: [name]
              properties:
                name:
                  type: string
      responses:
        '200': { description: ok }
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let tools = mcp_tools(&api, None).unwrap();
        let tool = &tools[0];
        let schema: Value = serde_json::from_str(&tool.input_schema).unwrap();

        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("body")));
        assert_eq!(
            schema["properties"]["body"]["type"],
            json!(["object", "null"])
        );
        assert!(tool.args.iter().any(|arg| arg.json_name == "body"
            && arg.required
            && matches!(arg.binding, McpArgBinding::WholeJsonBody)));
        assert!(!has_flattened_body_field(tool, "name"));
    }

    #[test]
    fn required_empty_object_request_body_uses_whole_json_body_arg() {
        let spec = r#"
openapi: 3.0.0
info: { title: Empty Root Body API, version: '1.0' }
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
              properties: {}
      responses:
        '200': { description: ok }
"#;
        let api = crate::spec::parse_spec_for_tests(spec).unwrap();
        let tools = mcp_tools(&api, None).unwrap();
        let tool = &tools[0];
        let schema: Value = serde_json::from_str(&tool.input_schema).unwrap();

        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("body")));
        assert_eq!(schema["properties"]["body"]["type"], "object");
        assert!(tool.args.iter().any(|arg| arg.json_name == "body"
            && arg.required
            && matches!(arg.binding, McpArgBinding::WholeJsonBody)));
        assert_eq!(tool.args.len(), 1);
    }
}
