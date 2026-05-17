use crate::backend::{DirectInvocationParameterLocation, DirectInvocationRequirements};
use anyhow::{bail, Result};
use openapiv3::{
    OpenAPI, Parameter, ParameterData, ParameterSchemaOrContent, PathStyle, QueryStyle,
    ReferenceOr, RequestBody,
};
use serde::Serialize;
use serde_json::{Map, Value};

use super::response::reject_reserved_arg;
use super::schema::{schema_projection, ProjectedSchema, SchemaPrimitive, SchemaShape};

pub(super) const DIRECT_UNSUPPORTED_PREFIX: &str = "MCP direct HTTP invocation does not support";

pub(super) struct McpArgumentContext<'a> {
    pub api: &'a OpenAPI,
    pub capabilities: &'a DirectInvocationRequirements,
    pub tool_name: &'a str,
    pub operation_id: &'a str,
}

pub type McpArg = GeneratedArg;
pub type McpArgBinding = GeneratedArgBinding;

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedArg {
    pub json_name: String,
    pub binding: GeneratedArgBinding,
    pub required: bool,
    pub value_kind: ArgValueKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GeneratedArgBinding {
    PathParam { wire_name: String },
    QueryParam { wire_name: String },
    FlattenedBodyField,
    WholeJsonBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArgValueKind {
    String,
    Integer,
    Number,
    Boolean,
    PrimitiveArray { item: PrimitiveKind },
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveKind {
    String,
    Integer,
    Number,
    Boolean,
}

impl GeneratedArg {
    pub(super) fn path_param(
        json_name: String,
        wire_name: String,
        required: bool,
        value_kind: ArgValueKind,
    ) -> Self {
        Self {
            json_name,
            binding: GeneratedArgBinding::PathParam { wire_name },
            required,
            value_kind,
        }
    }

    pub(super) fn query_param(
        json_name: String,
        wire_name: String,
        required: bool,
        value_kind: ArgValueKind,
    ) -> Self {
        Self {
            json_name,
            binding: GeneratedArgBinding::QueryParam { wire_name },
            required,
            value_kind,
        }
    }

    pub(super) fn flattened_body_field(
        json_name: String,
        required: bool,
        value_kind: ArgValueKind,
    ) -> Self {
        Self {
            json_name,
            binding: GeneratedArgBinding::FlattenedBodyField,
            required,
            value_kind,
        }
    }

    pub(super) fn whole_json_body(json_name: String, required: bool) -> Self {
        Self {
            json_name,
            binding: GeneratedArgBinding::WholeJsonBody,
            required,
            value_kind: ArgValueKind::Json,
        }
    }
}

fn reject_reserved_cli_arg(name: &str, tool_name: &str, operation_id: &str) -> Result<()> {
    if matches!(name, "json" | "help") {
        anyhow::bail!(
            "MCP/CLI argument collision for tool '{tool_name}' (operationId '{operation_id}'): argument '{name}' is reserved by the generated CLI"
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

pub(super) fn add_parameter(
    parameter: &ReferenceOr<Parameter>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    ctx: &McpArgumentContext<'_>,
) -> Result<()> {
    let api = ctx.api;
    let capabilities = ctx.capabilities;
    let tool_name = ctx.tool_name;
    let operation_id = ctx.operation_id;
    let ReferenceOr::Item(parameter) = parameter else {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} unresolved parameter references for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    let (data, is_path) = match parameter {
        Parameter::Query { parameter_data, .. } => {
            reject_unsupported_parameter_location(
                capabilities,
                DirectInvocationParameterLocation::Query,
                "query",
                &parameter_data.name,
                tool_name,
                operation_id,
            )?;
            (parameter_data, false)
        }
        Parameter::Path { parameter_data, .. } => {
            reject_unsupported_parameter_location(
                capabilities,
                DirectInvocationParameterLocation::Path,
                "path",
                &parameter_data.name,
                tool_name,
                operation_id,
            )?;
            (parameter_data, true)
        }
        Parameter::Header { parameter_data, .. } => {
            bail!(
                "{DIRECT_UNSUPPORTED_PREFIX} header parameter '{}' for tool '{tool_name}' (operationId '{operation_id}')",
                parameter_data.name
            );
        }
        Parameter::Cookie { parameter_data, .. } => {
            bail!(
                "{DIRECT_UNSUPPORTED_PREFIX} cookie parameter '{}' for tool '{tool_name}' (operationId '{operation_id}')",
                parameter_data.name
            );
        }
    };
    reject_reserved_arg(&data.name, tool_name, operation_id)?;
    reject_reserved_cli_arg(&data.name, tool_name, operation_id)?;
    reject_duplicate_arg(
        properties,
        &data.name,
        tool_name,
        operation_id,
        "OpenAPI parameter",
    )?;
    let schema = parameter_schema(data, api, tool_name, operation_id)?;
    reject_unsupported_direct_parameter_schema(
        &schema,
        &data.name,
        tool_name,
        operation_id,
        is_path,
        capabilities,
    )?;
    reject_unsupported_direct_parameter_serialization(
        parameter,
        &schema,
        &data.name,
        tool_name,
        operation_id,
        capabilities,
    )?;
    properties.insert(data.name.clone(), schema.json);
    let arg_required = is_path || data.required;
    if arg_required {
        required.push(data.name.clone());
    }
    let value_kind = value_kind_for_shape(&schema.shape);
    if is_path {
        args.push(McpArg::path_param(
            data.name.clone(),
            data.name.clone(),
            arg_required,
            value_kind,
        ));
    } else {
        args.push(McpArg::query_param(
            data.name.clone(),
            data.name.clone(),
            arg_required,
            value_kind,
        ));
    }
    Ok(())
}

fn parameter_schema(
    data: &ParameterData,
    api: &OpenAPI,
    tool_name: &str,
    operation_id: &str,
) -> Result<ProjectedSchema> {
    match &data.format {
        ParameterSchemaOrContent::Schema(schema) => Ok(schema_projection(schema, api)),
        ParameterSchemaOrContent::Content(_) => bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} content-encoded parameter '{}' for tool '{tool_name}' (operationId '{operation_id}')",
            data.name
        ),
    }
}

fn reject_unsupported_parameter_location(
    capabilities: &DirectInvocationRequirements,
    location: DirectInvocationParameterLocation,
    label: &str,
    name: &str,
    tool_name: &str,
    operation_id: &str,
) -> Result<()> {
    if capabilities
        .parameters
        .supported_locations
        .contains(&location)
    {
        return Ok(());
    }
    bail!(
        "{DIRECT_UNSUPPORTED_PREFIX} {label} parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
    );
}

fn reject_unsupported_direct_parameter_schema(
    schema: &ProjectedSchema,
    name: &str,
    tool_name: &str,
    operation_id: &str,
    is_path: bool,
    capabilities: &DirectInvocationRequirements,
) -> Result<()> {
    let Some(schema_type) = schema.shape.json_type() else {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} parameter '{name}' without primitive schema type for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    if capabilities
        .parameters
        .primitive_schema_types
        .contains(&schema_type)
    {
        return Ok(());
    }
    match &schema.shape {
        SchemaShape::Array { items } if !is_path && capabilities.parameters.supports_query_arrays => {
            match items.as_deref().and_then(SchemaShape::primitive_json_type) {
                Some(item_type)
                    if capabilities
                        .parameters
                        .primitive_schema_types
                        .contains(&item_type) => Ok(()),
                _ => bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} non-primitive array parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
                ),
            }
        }
        SchemaShape::Array { .. } if is_path => bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} array path parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
        ),
        _ => bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} {schema_type} parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
        ),
    }
}

fn reject_unsupported_direct_parameter_serialization(
    parameter: &Parameter,
    schema: &ProjectedSchema,
    name: &str,
    tool_name: &str,
    operation_id: &str,
    capabilities: &DirectInvocationRequirements,
) -> Result<()> {
    match parameter {
        Parameter::Query {
            parameter_data,
            style,
            ..
        } => {
            if capabilities.parameters.requires_form_query_style && *style != QueryStyle::Form {
                bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} non-form query parameter serialization for '{name}' on tool '{tool_name}' (operationId '{operation_id}')"
                );
            }
            if matches!(&schema.shape, SchemaShape::Array { .. })
                && !capabilities.parameters.supports_non_exploded_query_arrays
                && parameter_data.explode == Some(false)
            {
                bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} non-exploded query array parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
                );
            }
            Ok(())
        }
        Parameter::Path {
            parameter_data,
            style,
        } => {
            if capabilities.parameters.requires_simple_path_style
                && (*style != PathStyle::Simple || parameter_data.explode == Some(true))
            {
                bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} non-simple path parameter serialization for '{name}' on tool '{tool_name}' (operationId '{operation_id}')"
                );
            }
            Ok(())
        }
        Parameter::Header { .. } | Parameter::Cookie { .. } => Ok(()),
    }
}

pub(super) fn add_body(
    request_body: Option<&ReferenceOr<RequestBody>>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    ctx: &McpArgumentContext<'_>,
) -> Result<()> {
    let api = ctx.api;
    let capabilities = ctx.capabilities;
    let tool_name = ctx.tool_name;
    let operation_id = ctx.operation_id;
    let Some(request_body) = request_body else {
        return Ok(());
    };
    let ReferenceOr::Item(body) = request_body else {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} unresolved requestBody references for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    let Some(media_type) = body
        .content
        .get(capabilities.request_bodies.json_content_type)
    else {
        if body.content.is_empty() {
            bail!(
                "{DIRECT_UNSUPPORTED_PREFIX} requestBody without JSON content for tool '{tool_name}' (operationId '{operation_id}')"
            );
        }
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} non-JSON request bodies for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    let Some(schema) = media_type.schema.as_ref() else {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} schemaless JSON request body for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    let body_schema = schema_projection(schema, api);
    if let SchemaShape::Object {
        properties: body_properties,
        required: body_required,
        flattenable: true,
    } = &body_schema.shape
    {
        let has_flattening_collision = body_properties
            .keys()
            .any(|name| properties.contains_key(name));
        if has_flattening_collision {
            bail!(
                "{DIRECT_UNSUPPORTED_PREFIX} flattened JSON request body field collision for tool '{tool_name}' (operationId '{operation_id}')"
            );
        }

        for (name, property_schema) in body_properties {
            reject_reserved_arg(name, tool_name, operation_id)?;
            reject_reserved_cli_arg(name, tool_name, operation_id)?;
            properties.insert(name.clone(), property_schema.json.clone());
            args.push(McpArg::flattened_body_field(
                name.clone(),
                body.required && body_required.contains(name),
                value_kind_for_shape(&property_schema.shape),
            ));
        }
        if body.required {
            required.extend(body_required.iter().cloned());
        }
        return Ok(());
    }
    add_synthetic_body_arg(
        body_schema.json,
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
    reject_reserved_cli_arg("body", tool_name, operation_id)?;
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
    args.push(McpArg::whole_json_body("body".to_string(), body_required));
    Ok(())
}

fn value_kind_for_shape(shape: &SchemaShape) -> ArgValueKind {
    match shape {
        SchemaShape::Primitive(primitive) => ArgValueKind::from(*primitive),
        SchemaShape::Array { items } => items
            .as_deref()
            .and_then(PrimitiveKind::from_shape)
            .map(|item| ArgValueKind::PrimitiveArray { item })
            .unwrap_or(ArgValueKind::Json),
        SchemaShape::Object { .. } | SchemaShape::Unknown => ArgValueKind::Json,
    }
}

impl From<SchemaPrimitive> for ArgValueKind {
    fn from(value: SchemaPrimitive) -> Self {
        match value {
            SchemaPrimitive::String => Self::String,
            SchemaPrimitive::Number => Self::Number,
            SchemaPrimitive::Integer => Self::Integer,
            SchemaPrimitive::Boolean => Self::Boolean,
        }
    }
}

impl PrimitiveKind {
    fn from_shape(shape: &SchemaShape) -> Option<Self> {
        match shape {
            SchemaShape::Primitive(SchemaPrimitive::String) => Some(Self::String),
            SchemaShape::Primitive(SchemaPrimitive::Number) => Some(Self::Number),
            SchemaShape::Primitive(SchemaPrimitive::Integer) => Some(Self::Integer),
            SchemaShape::Primitive(SchemaPrimitive::Boolean) => Some(Self::Boolean),
            _ => None,
        }
    }
}
