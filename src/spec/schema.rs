use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

use super::diagnostics::{SchemaFeature, UnsupportedSchemaDiagnostic};
use super::json_pointer::{encode_json_pointer_segment, resolve_local_ref};
use super::model::PpSpec;

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
    pub(crate) unsupported: Option<UnsupportedSchemaDiagnostic>,
}

impl ProjectedSchema {
    pub(crate) fn unsupported_diagnostic(&self) -> Option<&UnsupportedSchemaDiagnostic> {
        self.unsupported.as_ref()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum SchemaShape {
    Primitive(SchemaPrimitive),
    Array {
        items: Option<Box<SchemaShape>>,
        item_nullable: bool,
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
        return unsupported(pointer, SchemaFeature::BooleanOrNonObjectSchema);
    };

    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        if object
            .keys()
            .any(|key| key != "$ref" && !is_annotation_keyword(key))
        {
            return unsupported(pointer, SchemaFeature::RefSiblings);
        }
        return resolve_schema_reference(reference, scope_root, spec, stack, pointer);
    }

    for feature in unsupported_keywords() {
        if object.contains_key(*feature) {
            return unsupported(
                pointer,
                SchemaFeature::SchemaKeyword((*feature).to_string()),
            );
        }
    }

    match object.get("additionalProperties") {
        Some(Value::Bool(false)) | None => {}
        Some(_) => {
            return unsupported(
                pointer,
                SchemaFeature::SchemaKeyword("additionalProperties".to_string()),
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
        Some(other) => unsupported(
            pointer,
            SchemaFeature::UnsupportedJsonSchemaType(other.to_string()),
        ),
        None => unsupported(pointer, SchemaFeature::MissingSupportedType),
    }
}

fn resolve_schema_reference(
    reference: &str,
    scope_root: &Value,
    spec: &PpSpec,
    stack: &mut BTreeSet<String>,
    pointer: &str,
) -> ProjectedSchema {
    let (value, next_scope_root) = if let Some(value) = resolve_local_ref(scope_root, reference) {
        (value, scope_root)
    } else if let Some(value) = resolve_local_ref(spec.document(), reference) {
        (value, value)
    } else {
        return unsupported(
            pointer,
            SchemaFeature::UnresolvedReference(reference.to_string()),
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
            unsupported: None,
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
        return unsupported(pointer, SchemaFeature::TupleArrayItems);
    }
    let mut value = schema_type_json("array", nullable);
    let mut unsupported = None;
    let mut item_nullable = false;
    let items = object.get("items").map(|items| {
        let child_pointer = format!("{pointer}/items");
        let projected = project_schema_value(items, &child_pointer, scope_root, spec, stack);
        unsupported = projected.unsupported.clone();
        item_nullable = projected.nullable;
        value["items"] = projected.json;
        Box::new(projected.shape)
    });
    ProjectedSchema {
        json: value,
        shape: SchemaShape::Array {
            items,
            item_nullable,
        },
        nullable,
        unsupported,
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
    let mut unsupported = None;
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, schema) in properties {
            let child_pointer =
                format!("{pointer}/properties/{}", encode_json_pointer_segment(name));
            let projected = project_schema_value(schema, &child_pointer, scope_root, spec, stack);
            if unsupported.is_none() {
                unsupported = projected.unsupported.clone();
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
        unsupported,
    }
}

fn primitive_projection(primitive: SchemaPrimitive, nullable: bool) -> ProjectedSchema {
    ProjectedSchema {
        json: schema_type_json(primitive.as_json_type(), nullable),
        shape: SchemaShape::Primitive(primitive),
        nullable,
        unsupported: None,
    }
}

fn unsupported(pointer: &str, feature: SchemaFeature) -> ProjectedSchema {
    ProjectedSchema {
        json: json!({}),
        shape: SchemaShape::Unknown,
        nullable: false,
        unsupported: Some(UnsupportedSchemaDiagnostic::new(feature, pointer)),
    }
}

fn parse_schema_type(
    value: Option<&Value>,
    _pointer: &str,
) -> Result<(Option<String>, bool), SchemaFeature> {
    match value {
        Some(Value::String(value)) => Ok((Some(value.clone()), false)),
        Some(Value::Array(values)) => {
            let mut non_null = Vec::new();
            let mut has_null = false;
            for value in values {
                match value.as_str() {
                    Some("null") => has_null = true,
                    Some(other) => non_null.push(other.to_string()),
                    None => return Err(SchemaFeature::InvalidTypeArray),
                }
            }
            if has_null && non_null.len() == 1 {
                Ok((Some(non_null.remove(0)), true))
            } else {
                Err(SchemaFeature::UnsupportedTypeUnion)
            }
        }
        Some(_) => Err(SchemaFeature::InvalidType),
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
