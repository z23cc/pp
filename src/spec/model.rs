use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub(crate) struct PpSpec {
    doc: Value,
}

impl PpSpec {
    pub(crate) fn new(doc: Value) -> Self {
        Self { doc }
    }

    pub(crate) fn document(&self) -> &Value {
        &self.doc
    }

    pub(crate) fn document_mut(&mut self) -> &mut Value {
        &mut self.doc
    }

    pub(crate) fn title(&self) -> &str {
        self.doc
            .pointer("/info/title")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub(crate) fn first_server_url(&self) -> Option<&str> {
        self.doc
            .get("servers")
            .and_then(Value::as_array)
            .and_then(|servers| servers.first())
            .and_then(|server| server.get("url"))
            .and_then(Value::as_str)
    }

    pub(crate) fn operation_count(&self) -> usize {
        crate::spec::traversal::operations(self).len()
    }

    pub(crate) fn resolve_pointer(&self, reference: &str) -> Option<&Value> {
        let pointer = reference.strip_prefix('#')?;
        self.doc.pointer(pointer)
    }

    pub(crate) fn root_security_requirements(&self) -> Option<Vec<Vec<String>>> {
        security_requirement_names(self.doc.get("security")?)
    }

    #[cfg(test)]
    pub(crate) fn retain_paths_for_tests(&mut self, mut keep: impl FnMut(&str) -> bool) {
        if let Some(paths) = self.doc.get_mut("paths").and_then(Value::as_object_mut) {
            paths.retain(|path, _| keep(path));
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OperationRef<'a> {
    pub(crate) method: &'static str,
    pub(crate) method_uppercase: &'static str,
    pub(crate) path: &'a str,
    path_parameters: Vec<PpParameterRef<'a>>,
    operation: &'a Value,
}

impl<'a> OperationRef<'a> {
    pub(crate) fn new(
        method: &'static str,
        method_uppercase: &'static str,
        path: &'a str,
        path_parameters: Vec<PpParameterRef<'a>>,
        operation: &'a Value,
    ) -> Self {
        Self {
            method,
            method_uppercase,
            path,
            path_parameters,
            operation,
        }
    }

    pub(crate) fn explicit_operation_id(&self) -> Option<&'a str> {
        self.operation
            .get("operationId")
            .and_then(Value::as_str)
            .filter(|operation_id| !operation_id.trim().is_empty())
    }

    pub(crate) fn raw_operation_id(&self) -> Option<String> {
        self.operation
            .get("operationId")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    pub(crate) fn tags(&self) -> Vec<String> {
        self.operation
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    pub(crate) fn summary_or_description(&self) -> Option<&'a str> {
        self.operation
            .get("summary")
            .and_then(Value::as_str)
            .or_else(|| self.operation.get("description").and_then(Value::as_str))
    }

    pub(crate) fn parameters(&self) -> Vec<PpParameterRef<'a>> {
        let mut parameters = self.path_parameters.clone();
        parameters.extend(
            self.operation
                .get("parameters")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(PpParameterRef::new),
        );
        parameters
    }

    pub(crate) fn request_body(&self) -> Option<PpRequestBodyRef<'a>> {
        self.operation.get("requestBody").map(PpRequestBodyRef::new)
    }

    pub(crate) fn security_requirement_names(&self) -> Option<Vec<Vec<String>>> {
        security_requirement_names(self.operation.get("security")?)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PpParameterRef<'a>(&'a Value);

#[derive(Debug, Clone, Copy)]
pub(crate) struct PpParameter<'a>(&'a Value);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PpParameterLocation {
    Query,
    Path,
    Header,
    Cookie,
}

impl<'a> PpParameterRef<'a> {
    pub(crate) fn new(value: &'a Value) -> Self {
        Self(value)
    }

    pub(crate) fn item(self) -> Option<PpParameter<'a>> {
        if self.0.get("$ref").is_some() {
            None
        } else {
            Some(PpParameter(self.0))
        }
    }
}

impl<'a> PpParameter<'a> {
    pub(crate) fn location(&self) -> Option<PpParameterLocation> {
        match self.0.get("in").and_then(Value::as_str) {
            Some("query") => Some(PpParameterLocation::Query),
            Some("path") => Some(PpParameterLocation::Path),
            Some("header") => Some(PpParameterLocation::Header),
            Some("cookie") => Some(PpParameterLocation::Cookie),
            _ => None,
        }
    }

    pub(crate) fn name(&self) -> Option<&str> {
        self.0
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
    }

    pub(crate) fn required(&self) -> bool {
        self.0
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub(crate) fn schema(&self) -> Option<PpSchemaRef<'a>> {
        self.0.get("schema").map(|schema| {
            PpSchemaRef::new(
                schema,
                format!("parameter '{}' schema", self.name().unwrap_or("<unnamed>")),
                schema,
            )
        })
    }

    pub(crate) fn has_content_format(&self) -> bool {
        self.0.get("content").is_some()
    }

    pub(crate) fn query_style_is_form(&self) -> bool {
        self.0
            .get("style")
            .and_then(Value::as_str)
            .map(|style| style == "form")
            .unwrap_or(true)
    }

    pub(crate) fn query_explode_is_false(&self) -> bool {
        self.0.get("explode").and_then(Value::as_bool) == Some(false)
    }

    pub(crate) fn path_style_is_simple(&self) -> bool {
        self.0
            .get("style")
            .and_then(Value::as_str)
            .map(|style| style == "simple")
            .unwrap_or(true)
    }

    pub(crate) fn path_explode_is_true(&self) -> bool {
        self.0.get("explode").and_then(Value::as_bool) == Some(true)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PpRequestBodyRef<'a>(&'a Value);

#[derive(Debug, Clone, Copy)]
pub(crate) struct PpRequestBody<'a>(&'a Value);

impl<'a> PpRequestBodyRef<'a> {
    pub(crate) fn new(value: &'a Value) -> Self {
        Self(value)
    }

    pub(crate) fn item(self) -> Option<PpRequestBody<'a>> {
        if self.0.get("$ref").is_some() {
            None
        } else {
            Some(PpRequestBody(self.0))
        }
    }
}

impl<'a> PpRequestBody<'a> {
    pub(crate) fn required(&self) -> bool {
        self.0
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub(crate) fn content_is_empty(&self) -> bool {
        self.0
            .get("content")
            .and_then(Value::as_object)
            .map(Map::is_empty)
            .unwrap_or(true)
    }

    pub(crate) fn has_content_type(&self, content_type: &str) -> bool {
        self.0
            .get("content")
            .and_then(Value::as_object)
            .map(|content| content.contains_key(content_type))
            .unwrap_or(false)
    }

    pub(crate) fn schema_for_content_type(&self, content_type: &str) -> Option<PpSchemaRef<'a>> {
        self.0
            .pointer(&format!(
                "/content/{}/schema",
                encode_json_pointer_segment(content_type)
            ))
            .map(|schema| {
                PpSchemaRef::new(
                    schema,
                    format!("requestBody content '{content_type}' schema"),
                    schema,
                )
            })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PpSchemaRef<'a> {
    value: &'a Value,
    pointer: String,
    scope_root: &'a Value,
}

impl<'a> PpSchemaRef<'a> {
    pub(crate) fn new(value: &'a Value, pointer: String, scope_root: &'a Value) -> Self {
        Self {
            value,
            pointer,
            scope_root,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectedSchema {
    pub(crate) json: Value,
    pub(crate) shape: SchemaShape,
    pub(crate) nullable: bool,
    pub(crate) unsupported_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum SchemaShape {
    Primitive(SchemaPrimitive),
    Array {
        items: Option<Box<SchemaShape>>,
    },
    Object {
        properties: BTreeMap<String, ProjectedSchema>,
        required: Vec<String>,
        flattenable: bool,
    },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaPrimitive {
    String,
    Number,
    Integer,
    Boolean,
}

impl SchemaPrimitive {
    pub(crate) const fn as_json_type(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
        }
    }
}

impl SchemaShape {
    pub(crate) const fn json_type(&self) -> Option<&'static str> {
        match self {
            Self::Primitive(primitive) => Some(primitive.as_json_type()),
            Self::Array { .. } => Some("array"),
            Self::Object { .. } => Some("object"),
            Self::Unknown => None,
        }
    }

    pub(crate) fn primitive_json_type(&self) -> Option<&'static str> {
        match self {
            Self::Primitive(primitive) => Some(primitive.as_json_type()),
            _ => None,
        }
    }
}

pub(crate) fn schema_projection(schema: PpSchemaRef<'_>, spec: &PpSpec) -> ProjectedSchema {
    schema_projection_with_stack(schema, spec, &mut BTreeSet::new())
}

fn schema_projection_with_stack(
    schema: PpSchemaRef<'_>,
    spec: &PpSpec,
    stack: &mut BTreeSet<String>,
) -> ProjectedSchema {
    project_schema_value(
        schema.value,
        &schema.pointer,
        schema.scope_root,
        spec,
        stack,
    )
}

fn project_schema_value(
    schema: &Value,
    pointer: &str,
    scope_root: &Value,
    spec: &PpSpec,
    stack: &mut BTreeSet<String>,
) -> ProjectedSchema {
    let Some(object) = schema.as_object() else {
        return unsupported(pointer, "boolean/non-object schemas are not supported");
    };

    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        if object
            .keys()
            .any(|key| key != "$ref" && !is_annotation_keyword(key))
        {
            return unsupported(pointer, "schemas with $ref siblings are not supported");
        }
        return resolve_schema_reference(reference, scope_root, spec, stack, pointer);
    }

    for feature in unsupported_keywords() {
        if object.contains_key(*feature) {
            return unsupported(
                pointer,
                format!("unsupported JSON Schema feature '{feature}'"),
            );
        }
    }

    match object.get("additionalProperties") {
        Some(Value::Bool(false)) | None => {}
        Some(_) => {
            return unsupported(
                pointer,
                "unsupported JSON Schema feature 'additionalProperties'",
            );
        }
    }

    let (schema_type, nullable) = match parse_schema_type(object.get("type"), pointer) {
        Ok(value) => value,
        Err(reason) => return unsupported(pointer, reason),
    };

    match schema_type.as_deref() {
        Some("string") => primitive_projection(SchemaPrimitive::String, nullable),
        Some("number") => primitive_projection(SchemaPrimitive::Number, nullable),
        Some("integer") => primitive_projection(SchemaPrimitive::Integer, nullable),
        Some("boolean") => primitive_projection(SchemaPrimitive::Boolean, nullable),
        Some("array") => array_projection(object, pointer, scope_root, spec, stack, nullable),
        Some("object") => object_projection(object, pointer, scope_root, spec, stack, nullable),
        Some(other) => unsupported(pointer, format!("unsupported JSON Schema type '{other}'")),
        None => unsupported(
            pointer,
            "schema without primitive schema type or supported object/array type",
        ),
    }
}

fn resolve_schema_reference(
    reference: &str,
    scope_root: &Value,
    spec: &PpSpec,
    stack: &mut BTreeSet<String>,
    pointer: &str,
) -> ProjectedSchema {
    let Some(local_pointer) = reference.strip_prefix('#') else {
        return unsupported(
            pointer,
            format!("unresolved schema reference '{reference}'"),
        );
    };
    let (value, next_scope_root) = if let Some(value) = scope_root.pointer(local_pointer) {
        (value, scope_root)
    } else if let Some(value) = spec.resolve_pointer(reference) {
        (value, value)
    } else {
        return unsupported(
            pointer,
            format!("unresolved schema reference '{reference}'"),
        );
    };
    if !stack.insert(reference.to_string()) {
        return ProjectedSchema {
            json: json!({
                "type": "object",
                "description": format!("<recursive reference to {}>", reference.rsplit('/').next().unwrap_or(reference))
            }),
            shape: SchemaShape::Object {
                properties: BTreeMap::new(),
                required: Vec::new(),
                flattenable: false,
            },
            nullable: false,
            unsupported_reason: None,
        };
    }
    let projected = project_schema_value(value, reference, next_scope_root, spec, stack);
    stack.remove(reference);
    projected
}

fn array_projection(
    object: &Map<String, Value>,
    pointer: &str,
    scope_root: &Value,
    spec: &PpSpec,
    stack: &mut BTreeSet<String>,
    nullable: bool,
) -> ProjectedSchema {
    if matches!(object.get("items"), Some(Value::Array(_))) {
        return unsupported(pointer, "unsupported JSON Schema tuple array items");
    }
    let mut value = schema_type_json("array", nullable);
    let mut unsupported_reason = None;
    let items = object.get("items").map(|items| {
        let child_pointer = format!("{pointer}/items");
        let projected = project_schema_value(items, &child_pointer, scope_root, spec, stack);
        unsupported_reason = projected.unsupported_reason.clone();
        value["items"] = projected.json;
        Box::new(projected.shape)
    });
    ProjectedSchema {
        json: value,
        shape: SchemaShape::Array { items },
        nullable,
        unsupported_reason,
    }
}

fn object_projection(
    object: &Map<String, Value>,
    pointer: &str,
    scope_root: &Value,
    spec: &PpSpec,
    stack: &mut BTreeSet<String>,
    nullable: bool,
) -> ProjectedSchema {
    let mut json_properties = Map::new();
    let mut projected_properties = BTreeMap::new();
    let mut unsupported_reason = None;
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, schema) in properties {
            let child_pointer =
                format!("{pointer}/properties/{}", encode_json_pointer_segment(name));
            let projected = project_schema_value(schema, &child_pointer, scope_root, spec, stack);
            if unsupported_reason.is_none() {
                unsupported_reason = projected.unsupported_reason.clone();
            }
            json_properties.insert(name.clone(), projected.json.clone());
            projected_properties.insert(name.clone(), projected);
        }
    }
    let required = object
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut json = schema_type_json("object", nullable);
    json["properties"] = Value::Object(json_properties);
    json["required"] = json!(required);
    ProjectedSchema {
        json,
        shape: SchemaShape::Object {
            properties: projected_properties,
            required,
            flattenable: true,
        },
        nullable,
        unsupported_reason,
    }
}

fn primitive_projection(primitive: SchemaPrimitive, nullable: bool) -> ProjectedSchema {
    ProjectedSchema {
        json: schema_type_json(primitive.as_json_type(), nullable),
        shape: SchemaShape::Primitive(primitive),
        nullable,
        unsupported_reason: None,
    }
}

fn unsupported(pointer: &str, reason: impl Into<String>) -> ProjectedSchema {
    let reason = format!("{} at {pointer}", reason.into());
    ProjectedSchema {
        json: json!({}),
        shape: SchemaShape::Unknown,
        nullable: false,
        unsupported_reason: Some(reason),
    }
}

fn parse_schema_type(
    value: Option<&Value>,
    pointer: &str,
) -> Result<(Option<String>, bool), String> {
    match value {
        Some(Value::String(value)) => Ok((Some(value.clone()), false)),
        Some(Value::Array(values)) => {
            let mut non_null = Vec::new();
            let mut has_null = false;
            for value in values {
                match value.as_str() {
                    Some("null") => has_null = true,
                    Some(other) => non_null.push(other.to_string()),
                    None => return Err(format!("invalid JSON Schema type array at {pointer}")),
                }
            }
            if has_null && non_null.len() == 1 {
                Ok((Some(non_null.remove(0)), true))
            } else {
                Err("unsupported JSON Schema type union; only [T, null] is supported".to_string())
            }
        }
        Some(_) => Err("invalid JSON Schema type".to_string()),
        None => Ok((None, false)),
    }
}

fn schema_type_json(schema_type: &str, nullable: bool) -> Value {
    if nullable {
        json!({ "type": [schema_type, "null"] })
    } else {
        json!({ "type": schema_type })
    }
}

fn unsupported_keywords() -> &'static [&'static str] {
    &[
        "oneOf",
        "anyOf",
        "allOf",
        "not",
        "if",
        "then",
        "else",
        "dependentSchemas",
        "patternProperties",
        "prefixItems",
        "unevaluatedProperties",
        "unevaluatedItems",
        "contains",
        "propertyNames",
    ]
}

fn is_annotation_keyword(key: &str) -> bool {
    matches!(
        key,
        "title" | "description" | "default" | "examples" | "deprecated" | "readOnly" | "writeOnly"
    )
}

fn security_requirement_names(value: &Value) -> Option<Vec<Vec<String>>> {
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(Value::as_object)
            .map(|requirement| requirement.keys().cloned().collect())
            .collect(),
    )
}

pub(crate) fn encode_json_pointer_segment(input: &str) -> String {
    input.replace('~', "~0").replace('/', "~1")
}
