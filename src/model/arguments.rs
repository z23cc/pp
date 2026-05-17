use crate::backend::{DirectInvocationParameterLocation, DirectInvocationRequirements};
use crate::spec::{
    schema_projection, PpParameterLocation, PpParameterRef, PpRequestBodyRef, PpSpec,
    ProjectedSchema, SchemaPrimitive, SchemaShape,
};
use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::{Map, Value};

use super::response::reject_reserved_arg;

pub(super) const DIRECT_UNSUPPORTED_PREFIX: &str = "MCP direct HTTP invocation does not support";

pub(super) struct McpArgumentContext<'a> {
    pub spec: &'a PpSpec,
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
    NullablePrimitive { item: PrimitiveKind },
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
    parameter: PpParameterRef<'_>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    ctx: &McpArgumentContext<'_>,
) -> Result<()> {
    let spec = ctx.spec;
    let capabilities = ctx.capabilities;
    let tool_name = ctx.tool_name;
    let operation_id = ctx.operation_id;
    let Some(parameter) = parameter.item() else {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} unresolved parameter references for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    let name = parameter.name().ok_or_else(|| {
        anyhow::anyhow!(
            "{DIRECT_UNSUPPORTED_PREFIX} parameter without non-empty string name for tool '{tool_name}' (operationId '{operation_id}')"
        )
    })?;
    let location = parameter.location().ok_or_else(|| {
        anyhow::anyhow!(
            "{DIRECT_UNSUPPORTED_PREFIX} parameter '{name}' without supported 'in' location for tool '{tool_name}' (operationId '{operation_id}')"
        )
    })?;
    let is_path = match location {
        PpParameterLocation::Query => {
            reject_unsupported_parameter_location(
                capabilities,
                DirectInvocationParameterLocation::Query,
                "query",
                name,
                tool_name,
                operation_id,
            )?;
            false
        }
        PpParameterLocation::Path => {
            reject_unsupported_parameter_location(
                capabilities,
                DirectInvocationParameterLocation::Path,
                "path",
                name,
                tool_name,
                operation_id,
            )?;
            true
        }
        PpParameterLocation::Header => {
            bail!(
                "{DIRECT_UNSUPPORTED_PREFIX} header parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
            );
        }
        PpParameterLocation::Cookie => {
            bail!(
                "{DIRECT_UNSUPPORTED_PREFIX} cookie parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
            );
        }
    };
    reject_reserved_arg(name, tool_name, operation_id)?;
    reject_reserved_cli_arg(name, tool_name, operation_id)?;
    reject_duplicate_arg(
        properties,
        name,
        tool_name,
        operation_id,
        "OpenAPI parameter",
    )?;
    let schema = parameter_schema(parameter, name, spec, tool_name, operation_id)?;
    if schema.nullable && (is_path || parameter.required()) {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} required nullable parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
        );
    }
    reject_unsupported_direct_parameter_schema(
        &schema,
        name,
        tool_name,
        operation_id,
        is_path,
        capabilities,
    )?;
    reject_unsupported_direct_parameter_serialization(
        parameter,
        location,
        &schema,
        name,
        tool_name,
        operation_id,
        capabilities,
    )?;
    let value_kind = value_kind_for_schema(&schema);
    properties.insert(name.to_string(), schema.json);
    let arg_required = is_path || parameter.required();
    if arg_required {
        required.push(name.to_string());
    }
    if is_path {
        args.push(McpArg::path_param(
            name.to_string(),
            name.to_string(),
            arg_required,
            value_kind,
        ));
    } else {
        args.push(McpArg::query_param(
            name.to_string(),
            name.to_string(),
            arg_required,
            value_kind,
        ));
    }
    Ok(())
}

fn parameter_schema(
    parameter: crate::spec::PpParameter<'_>,
    name: &str,
    spec: &PpSpec,
    tool_name: &str,
    operation_id: &str,
) -> Result<ProjectedSchema> {
    if let Some(schema) = parameter.schema() {
        return Ok(schema_projection(schema, spec));
    }
    if parameter.has_content_format() {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} content-encoded parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
        );
    }
    bail!(
        "{DIRECT_UNSUPPORTED_PREFIX} parameter '{name}' without schema for tool '{tool_name}' (operationId '{operation_id}')"
    )
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
    if let Some(reason) = &schema.unsupported_reason {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} unsupported parameter schema for '{name}' on tool '{tool_name}' (operationId '{operation_id}'): {reason}"
        );
    }
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
    parameter: crate::spec::PpParameter<'_>,
    location: PpParameterLocation,
    schema: &ProjectedSchema,
    name: &str,
    tool_name: &str,
    operation_id: &str,
    capabilities: &DirectInvocationRequirements,
) -> Result<()> {
    match location {
        PpParameterLocation::Query => {
            if capabilities.parameters.requires_form_query_style && !parameter.query_style_is_form()
            {
                bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} non-form query parameter serialization for '{name}' on tool '{tool_name}' (operationId '{operation_id}')"
                );
            }
            if matches!(&schema.shape, SchemaShape::Array { .. })
                && !capabilities.parameters.supports_non_exploded_query_arrays
                && parameter.query_explode_is_false()
            {
                bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} non-exploded query array parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
                );
            }
            Ok(())
        }
        PpParameterLocation::Path => {
            if capabilities.parameters.requires_simple_path_style
                && (!parameter.path_style_is_simple() || parameter.path_explode_is_true())
            {
                bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} non-simple path parameter serialization for '{name}' on tool '{tool_name}' (operationId '{operation_id}')"
                );
            }
            Ok(())
        }
        PpParameterLocation::Header | PpParameterLocation::Cookie => Ok(()),
    }
}

pub(super) fn add_body(
    request_body: Option<PpRequestBodyRef<'_>>,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<McpArg>,
    ctx: &McpArgumentContext<'_>,
) -> Result<()> {
    let spec = ctx.spec;
    let capabilities = ctx.capabilities;
    let tool_name = ctx.tool_name;
    let operation_id = ctx.operation_id;
    let Some(request_body) = request_body else {
        return Ok(());
    };
    let Some(body) = request_body.item() else {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} unresolved requestBody references for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    let json_content_type = capabilities.request_bodies.json_content_type;
    if !body.has_content_type(json_content_type) {
        if body.content_is_empty() {
            bail!(
                "{DIRECT_UNSUPPORTED_PREFIX} requestBody without JSON content for tool '{tool_name}' (operationId '{operation_id}')"
            );
        }
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} non-JSON request bodies for tool '{tool_name}' (operationId '{operation_id}')"
        );
    }
    let Some(schema) = body.schema_for_content_type(json_content_type) else {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} schemaless JSON request body for tool '{tool_name}' (operationId '{operation_id}')"
        );
    };
    let body_schema = schema_projection(schema, spec);
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
            if let Some(reason) = &property_schema.unsupported_reason {
                bail!(
                    "{DIRECT_UNSUPPORTED_PREFIX} unsupported JSON request body field '{name}' for tool '{tool_name}' (operationId '{operation_id}'): {reason}"
                );
            }
            properties.insert(name.clone(), property_schema.json.clone());
            args.push(McpArg::flattened_body_field(
                name.clone(),
                body.required() && body_required.contains(name),
                value_kind_for_schema(property_schema),
            ));
        }
        if body.required() {
            required.extend(body_required.iter().cloned());
        }
        return Ok(());
    }
    if let Some(reason) = &body_schema.unsupported_reason {
        bail!(
            "{DIRECT_UNSUPPORTED_PREFIX} unsupported JSON request body schema for tool '{tool_name}' (operationId '{operation_id}'): {reason}"
        );
    }
    add_synthetic_body_arg(
        body_schema.json,
        body.required(),
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

fn value_kind_for_schema(schema: &ProjectedSchema) -> ArgValueKind {
    if schema.nullable {
        if let Some(item) = PrimitiveKind::from_shape(&schema.shape) {
            return ArgValueKind::NullablePrimitive { item };
        }
    }
    value_kind_for_shape(&schema.shape)
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
