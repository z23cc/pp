use crate::backend::{DirectInvocationParameterLocation, DirectInvocationRequirements};
use crate::spec::{
    schema_projection, OperationRef, PpParameter, PpParameterLocation, PpParameterRef,
    PpRequestBodyRef, PpSpec, ProjectedSchema, SchemaPrimitive, SchemaShape,
};
use crate::support::diagnostics::direct_http as direct_codes;
use anyhow::Result;
use serde::Serialize;
use serde_json::{Map, Value};

use super::arguments::GeneratedArg;
use super::diagnostics::DirectInvocationUnsupported;
use super::response::reject_reserved_arg;
use super::value_kind::{ArgValueKind, PrimitiveKind};

fn unsupported_error(code: &'static str, detail: impl Into<String>) -> anyhow::Error {
    DirectInvocationUnsupported::new(code, detail).into()
}

fn unsupported_schema_error(
    code: &'static str,
    detail: impl Into<String>,
    source_code: &'static str,
) -> anyhow::Error {
    DirectInvocationUnsupported::with_source_code(code, detail, source_code).into()
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct OperationInvocationPlan {
    pub properties: Map<String, Value>,
    pub required: Vec<String>,
    pub args: Vec<GeneratedArg>,
}

#[derive(Debug, Clone)]
pub(crate) struct OperationInvocationPlanRequest<'a> {
    pub spec: &'a PpSpec,
    pub operation: OperationRef<'a>,
    pub tool_name: &'a str,
    pub operation_id: &'a str,
}

pub(super) struct OperationInvocationPlanContext<'a> {
    pub spec: &'a PpSpec,
    pub capabilities: &'a DirectInvocationRequirements,
    pub tool_name: &'a str,
    pub operation_id: &'a str,
}

#[derive(Debug, Clone)]
pub(super) struct PlannedInvocationArg {
    pub json_name: String,
    pub wire_name: String,
    pub binding: InvocationArgBinding,
    pub required: bool,
    pub schema_json: Value,
    pub value_kind: ArgValueKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InvocationArgBinding {
    Path,
    Query,
}

#[derive(Debug, Clone)]
pub(super) enum PlannedInvocationBody {
    None,
    Flattened {
        fields: Vec<PlannedInvocationBodyField>,
        required: Vec<String>,
    },
    WholeJson {
        schema_json: Value,
        required: bool,
    },
}

#[derive(Debug, Clone)]
pub(super) struct PlannedInvocationBodyField {
    pub json_name: String,
    pub schema_json: Value,
    pub required: bool,
    pub value_kind: ArgValueKind,
}

pub(crate) fn plan_native_http_operation_invocation(
    request: OperationInvocationPlanRequest<'_>,
    capabilities: &DirectInvocationRequirements,
) -> Result<OperationInvocationPlan> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut args = Vec::new();
    let ctx = OperationInvocationPlanContext {
        spec: request.spec,
        capabilities,
        tool_name: request.tool_name,
        operation_id: request.operation_id,
    };

    for parameter in request.operation.parameters() {
        let planned = plan_parameter(parameter, &ctx, |name| {
            reject_reserved_arg(name, request.tool_name, request.operation_id)?;
            reject_reserved_cli_arg(name, request.tool_name, request.operation_id)?;
            reject_duplicate_arg(
                &properties,
                name,
                request.tool_name,
                request.operation_id,
                "OpenAPI parameter",
            )
        })?;

        properties.insert(planned.json_name.clone(), planned.schema_json);
        if planned.required {
            required.push(planned.json_name.clone());
        }
        match planned.binding {
            InvocationArgBinding::Path => args.push(GeneratedArg::path_param(
                planned.json_name,
                planned.wire_name,
                planned.required,
                planned.value_kind,
            )),
            InvocationArgBinding::Query => args.push(GeneratedArg::query_param(
                planned.json_name,
                planned.wire_name,
                planned.required,
                planned.value_kind,
            )),
        }
    }

    match plan_request_body(
        request.operation.request_body(),
        &ctx,
        |field_names| {
            if field_names.iter().any(|name| properties.contains_key(name)) {
                return Err(unsupported_error(
                    direct_codes::REQUEST_BODY_FIELD_COLLISION,
                    format!(
                        "flattened JSON request body field collision for tool '{}' (operationId '{}')",
                        request.tool_name, request.operation_id
                    ),
                ));
            }
            Ok(())
        },
        |field_name| {
            reject_reserved_arg(field_name, request.tool_name, request.operation_id)?;
            reject_reserved_cli_arg(field_name, request.tool_name, request.operation_id)
        },
    )? {
        PlannedInvocationBody::None => {}
        PlannedInvocationBody::Flattened {
            fields,
            required: body_required,
        } => {
            for field in fields {
                properties.insert(field.json_name.clone(), field.schema_json);
                args.push(GeneratedArg::flattened_body_field(
                    field.json_name,
                    field.required,
                    field.value_kind,
                ));
            }
            required.extend(body_required);
        }
        PlannedInvocationBody::WholeJson {
            schema_json,
            required: body_required,
        } => add_synthetic_body_arg(
            schema_json,
            body_required,
            &mut properties,
            &mut required,
            &mut args,
            request.tool_name,
            request.operation_id,
        )?,
    }

    Ok(OperationInvocationPlan {
        properties,
        required,
        args,
    })
}

pub(super) fn plan_parameter(
    parameter: PpParameterRef<'_>,
    ctx: &OperationInvocationPlanContext<'_>,
    validate_arg_name: impl FnOnce(&str) -> Result<()>,
) -> Result<PlannedInvocationArg> {
    let spec = ctx.spec;
    let capabilities = ctx.capabilities;
    let tool_name = ctx.tool_name;
    let operation_id = ctx.operation_id;
    let Some(parameter) = parameter.item() else {
        return Err(unsupported_error(
            direct_codes::UNRESOLVED_PARAMETER_REF,
            format!(
            "unresolved parameter references for tool '{tool_name}' (operationId '{operation_id}')"
        ),
        ));
    };
    let name = parameter.name().ok_or_else(|| {
        unsupported_error(direct_codes::PARAMETER_NAME_MISSING, format!(
            "parameter without non-empty string name for tool '{tool_name}' (operationId '{operation_id}')"
        ))
    })?;
    let location = parameter.location().ok_or_else(|| {
        unsupported_error(direct_codes::PARAMETER_LOCATION_MISSING, format!(
            "parameter '{name}' without supported 'in' location for tool '{tool_name}' (operationId '{operation_id}')"
        ))
    })?;
    let binding = match location {
        PpParameterLocation::Query => {
            reject_unsupported_parameter_location(
                capabilities,
                DirectInvocationParameterLocation::Query,
                "query",
                name,
                tool_name,
                operation_id,
            )?;
            InvocationArgBinding::Query
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
            InvocationArgBinding::Path
        }
        PpParameterLocation::Header => {
            return Err(unsupported_error(
                direct_codes::PARAMETER_LOCATION_UNSUPPORTED,
                format!(
                "header parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
            ),
            ));
        }
        PpParameterLocation::Cookie => {
            return Err(unsupported_error(
                direct_codes::PARAMETER_LOCATION_UNSUPPORTED,
                format!(
                "cookie parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
            ),
            ));
        }
    };
    validate_arg_name(name)?;
    let is_path = matches!(binding, InvocationArgBinding::Path);
    let schema = parameter_schema(parameter, name, spec, tool_name, operation_id)?;
    if schema.nullable && (is_path || parameter.required()) {
        return Err(unsupported_error(direct_codes::PARAMETER_REQUIRED_NULLABLE, format!(
            "required nullable parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
        )));
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
    Ok(PlannedInvocationArg {
        json_name: name.to_string(),
        wire_name: name.to_string(),
        binding,
        required: is_path || parameter.required(),
        schema_json: schema.json,
        value_kind,
    })
}

pub(super) fn plan_request_body(
    request_body: Option<PpRequestBodyRef<'_>>,
    ctx: &OperationInvocationPlanContext<'_>,
    validate_flattened_names: impl FnOnce(&[String]) -> Result<()>,
    mut validate_flattened_field: impl FnMut(&str) -> Result<()>,
) -> Result<PlannedInvocationBody> {
    let spec = ctx.spec;
    let capabilities = ctx.capabilities;
    let tool_name = ctx.tool_name;
    let operation_id = ctx.operation_id;
    let Some(request_body) = request_body else {
        return Ok(PlannedInvocationBody::None);
    };
    let Some(body) = request_body.item() else {
        return Err(unsupported_error(direct_codes::UNRESOLVED_REQUEST_BODY_REF, format!(
            "unresolved requestBody references for tool '{tool_name}' (operationId '{operation_id}')"
        )));
    };
    let json_content_type = capabilities.request_bodies.json_content_type;
    if !body.has_content_type(json_content_type) {
        if body.content_is_empty() {
            return Err(unsupported_error(direct_codes::REQUEST_BODY_JSON_MISSING, format!(
                "requestBody without JSON content for tool '{tool_name}' (operationId '{operation_id}')"
            )));
        }
        return Err(unsupported_error(
            direct_codes::REQUEST_BODY_NON_JSON,
            format!(
                "non-JSON request bodies for tool '{tool_name}' (operationId '{operation_id}')"
            ),
        ));
    }
    let Some(schema) = body.schema_for_content_type(json_content_type) else {
        return Err(unsupported_error(
            direct_codes::REQUEST_BODY_SCHEMA_MISSING,
            format!(
            "schemaless JSON request body for tool '{tool_name}' (operationId '{operation_id}')"
        ),
        ));
    };
    let body_schema = schema_projection(schema, spec);
    if let SchemaShape::Object {
        properties: body_properties,
        required: body_required,
        flattenable: true,
    } = &body_schema.shape
    {
        let mut fields = Vec::with_capacity(body_properties.len());
        for (name, property_schema) in body_properties {
            if let Some(diagnostic) = property_schema.unsupported_diagnostic() {
                let reason = diagnostic.to_string();
                return Err(unsupported_schema_error(
                    direct_codes::REQUEST_BODY_FIELD_SCHEMA_UNSUPPORTED,
                    format!(
                        "unsupported JSON request body field '{name}' for tool '{tool_name}' (operationId '{operation_id}'): {reason}"
                    ),
                    diagnostic.code(),
                ));
            }
            fields.push(PlannedInvocationBodyField {
                json_name: name.clone(),
                schema_json: property_schema.json.clone(),
                required: body.required() && body_required.contains(name),
                value_kind: value_kind_for_schema(property_schema),
            });
        }
        if !body_schema.nullable && !fields.is_empty() {
            let field_names = body_properties.keys().cloned().collect::<Vec<_>>();
            validate_flattened_names(&field_names)?;
            for name in body_properties.keys() {
                validate_flattened_field(name)?;
            }
            return Ok(PlannedInvocationBody::Flattened {
                fields,
                required: if body.required() {
                    body_required.clone()
                } else {
                    Vec::new()
                },
            });
        }
    }
    if let Some(diagnostic) = body_schema.unsupported_diagnostic() {
        let reason = diagnostic.to_string();
        return Err(unsupported_schema_error(
            direct_codes::REQUEST_BODY_SCHEMA_UNSUPPORTED,
            format!(
                "unsupported JSON request body schema for tool '{tool_name}' (operationId '{operation_id}'): {reason}"
            ),
            diagnostic.code(),
        ));
    }
    Ok(PlannedInvocationBody::WholeJson {
        schema_json: body_schema.json,
        required: body.required(),
    })
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

fn add_synthetic_body_arg(
    body_schema: Value,
    body_required: bool,
    properties: &mut Map<String, Value>,
    required: &mut Vec<String>,
    args: &mut Vec<GeneratedArg>,
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
    args.push(GeneratedArg::whole_json_body(
        "body".to_string(),
        body_required,
    ));
    Ok(())
}

fn parameter_schema(
    parameter: PpParameter<'_>,
    name: &str,
    spec: &PpSpec,
    tool_name: &str,
    operation_id: &str,
) -> Result<ProjectedSchema> {
    if let Some(schema) = parameter.schema() {
        return Ok(schema_projection(schema, spec));
    }
    if parameter.has_content_format() {
        return Err(unsupported_error(direct_codes::PARAMETER_CONTENT_ENCODING, format!(
            "content-encoded parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
        )));
    }
    Err(unsupported_error(
        direct_codes::PARAMETER_SCHEMA_MISSING,
        format!(
        "parameter '{name}' without schema for tool '{tool_name}' (operationId '{operation_id}')"
    ),
    ))
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
    Err(unsupported_error(
        direct_codes::PARAMETER_LOCATION_UNSUPPORTED,
        format!("{label} parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"),
    ))
}

fn reject_unsupported_direct_parameter_schema(
    schema: &ProjectedSchema,
    name: &str,
    tool_name: &str,
    operation_id: &str,
    is_path: bool,
    capabilities: &DirectInvocationRequirements,
) -> Result<()> {
    if let Some(diagnostic) = schema.unsupported_diagnostic() {
        let reason = diagnostic.to_string();
        return Err(unsupported_schema_error(
            direct_codes::PARAMETER_SCHEMA_UNSUPPORTED,
            format!(
                "unsupported parameter schema for '{name}' on tool '{tool_name}' (operationId '{operation_id}'): {reason}"
            ),
            diagnostic.code(),
        ));
    }
    let Some(schema_type) = schema.shape.json_type() else {
        return Err(unsupported_error(direct_codes::PARAMETER_PRIMITIVE_TYPE_MISSING, format!(
            "parameter '{name}' without primitive schema type for tool '{tool_name}' (operationId '{operation_id}')"
        )));
    };
    if capabilities
        .parameters
        .primitive_schema_types
        .contains(&schema_type)
    {
        return Ok(());
    }
    match &schema.shape {
        SchemaShape::Array {
            items,
            item_nullable,
        } if !is_path && capabilities.parameters.supports_query_arrays => {
            if *item_nullable {
                return Err(unsupported_error(direct_codes::QUERY_ARRAY_ITEM_NULLABLE, format!(
                    "nullable array items for parameter '{name}' on tool '{tool_name}' (operationId '{operation_id}')"
                )));
            }
            match items.as_deref().and_then(SchemaShape::primitive_json_type) {
                Some(item_type)
                    if capabilities
                        .parameters
                        .primitive_schema_types
                        .contains(&item_type) => Ok(()),
                _ => Err(unsupported_error(direct_codes::PARAMETER_ARRAY_NON_PRIMITIVE, format!(
                    "non-primitive array parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
                ))),
            }
        }
        SchemaShape::Array { .. } if is_path => Err(unsupported_error(
            direct_codes::PATH_ARRAY_UNSUPPORTED,
            format!(
            "array path parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
        ),
        )),
        _ => Err(unsupported_error(
            direct_codes::PARAMETER_TYPE_UNSUPPORTED,
            format!(
            "{schema_type} parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
        ),
        )),
    }
}

fn reject_unsupported_direct_parameter_serialization(
    parameter: PpParameter<'_>,
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
                return Err(unsupported_error(direct_codes::QUERY_STYLE_NON_FORM, format!(
                    "non-form query parameter serialization for '{name}' on tool '{tool_name}' (operationId '{operation_id}')"
                )));
            }
            if matches!(&schema.shape, SchemaShape::Array { .. })
                && !capabilities.parameters.supports_non_exploded_query_arrays
                && parameter.query_explode_is_false()
            {
                return Err(unsupported_error(direct_codes::QUERY_ARRAY_NON_EXPLODED, format!(
                    "non-exploded query array parameter '{name}' for tool '{tool_name}' (operationId '{operation_id}')"
                )));
            }
            Ok(())
        }
        PpParameterLocation::Path => {
            if capabilities.parameters.requires_simple_path_style
                && (!parameter.path_style_is_simple() || parameter.path_explode_is_true())
            {
                return Err(unsupported_error(direct_codes::PATH_STYLE_NON_SIMPLE, format!(
                    "non-simple path parameter serialization for '{name}' on tool '{tool_name}' (operationId '{operation_id}')"
                )));
            }
            Ok(())
        }
        PpParameterLocation::Header | PpParameterLocation::Cookie => Ok(()),
    }
}

fn value_kind_for_schema(schema: &ProjectedSchema) -> ArgValueKind {
    if schema.nullable {
        if let Some(item) = primitive_kind_for_shape(&schema.shape) {
            return ArgValueKind::NullablePrimitive { item };
        }
    }
    value_kind_for_shape(&schema.shape)
}

fn value_kind_for_shape(shape: &SchemaShape) -> ArgValueKind {
    match shape {
        SchemaShape::Primitive(primitive) => arg_value_kind_for_primitive(*primitive),
        SchemaShape::Array { items, .. } => items
            .as_deref()
            .and_then(primitive_kind_for_shape)
            .map(|item| ArgValueKind::PrimitiveArray { item })
            .unwrap_or(ArgValueKind::Json),
        SchemaShape::Object { .. } | SchemaShape::Unknown => ArgValueKind::Json,
    }
}

fn arg_value_kind_for_primitive(primitive: SchemaPrimitive) -> ArgValueKind {
    match primitive {
        SchemaPrimitive::String => ArgValueKind::String,
        SchemaPrimitive::Number => ArgValueKind::Number,
        SchemaPrimitive::Integer => ArgValueKind::Integer,
        SchemaPrimitive::Boolean => ArgValueKind::Boolean,
    }
}

fn primitive_kind_for_shape(shape: &SchemaShape) -> Option<PrimitiveKind> {
    match shape {
        SchemaShape::Primitive(SchemaPrimitive::String) => Some(PrimitiveKind::String),
        SchemaShape::Primitive(SchemaPrimitive::Number) => Some(PrimitiveKind::Number),
        SchemaShape::Primitive(SchemaPrimitive::Integer) => Some(PrimitiveKind::Integer),
        SchemaShape::Primitive(SchemaPrimitive::Boolean) => Some(PrimitiveKind::Boolean),
        _ => None,
    }
}
