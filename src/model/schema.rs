use openapiv3::{OpenAPI, ReferenceOr, Schema, SchemaKind, Type};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub(super) struct ProjectedSchema {
    pub(super) json: Value,
    pub(super) shape: SchemaShape,
}

#[derive(Debug, Clone)]
pub(super) enum SchemaShape {
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
pub(super) enum SchemaPrimitive {
    String,
    Number,
    Integer,
    Boolean,
}

impl SchemaPrimitive {
    pub(super) const fn as_json_type(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
        }
    }
}

impl SchemaShape {
    pub(super) const fn json_type(&self) -> Option<&'static str> {
        match self {
            Self::Primitive(primitive) => Some(primitive.as_json_type()),
            Self::Array { .. } => Some("array"),
            Self::Object { .. } => Some("object"),
            Self::Unknown => None,
        }
    }

    pub(super) fn primitive_json_type(&self) -> Option<&'static str> {
        match self {
            Self::Primitive(primitive) => Some(primitive.as_json_type()),
            _ => None,
        }
    }
}

pub(super) fn schema_projection(schema: &ReferenceOr<Schema>, api: &OpenAPI) -> ProjectedSchema {
    schema_projection_with_stack(schema, api, &mut BTreeSet::new())
}

fn schema_projection_with_stack(
    schema: &ReferenceOr<Schema>,
    api: &OpenAPI,
    stack: &mut BTreeSet<String>,
) -> ProjectedSchema {
    match schema {
        ReferenceOr::Reference { reference } => resolve_schema_reference(reference, api, stack),
        ReferenceOr::Item(schema) => schema_kind_projection(&schema.schema_kind, api, stack),
    }
}

fn boxed_schema_projection_with_stack(
    schema: &ReferenceOr<Box<Schema>>,
    api: &OpenAPI,
    stack: &mut BTreeSet<String>,
) -> ProjectedSchema {
    match schema {
        ReferenceOr::Reference { reference } => resolve_schema_reference(reference, api, stack),
        ReferenceOr::Item(schema) => schema_kind_projection(&schema.schema_kind, api, stack),
    }
}

fn resolve_schema_reference(
    reference: &str,
    api: &OpenAPI,
    stack: &mut BTreeSet<String>,
) -> ProjectedSchema {
    let Some(name) = reference.strip_prefix("#/components/schemas/") else {
        return ProjectedSchema {
            json: json!({ "$ref": reference }),
            shape: SchemaShape::Unknown,
        };
    };
    if !stack.insert(name.to_string()) {
        return ProjectedSchema {
            json: json!({
                "type": "object",
                "description": format!("<recursive reference to {name}>")
            }),
            shape: SchemaShape::Object {
                properties: BTreeMap::new(),
                required: Vec::new(),
                flattenable: false,
            },
        };
    }
    let value = api
        .components
        .as_ref()
        .and_then(|components| components.schemas.get(name))
        .map(|schema| schema_projection_with_stack(schema, api, stack))
        .unwrap_or_else(|| ProjectedSchema {
            json: json!({ "$ref": reference }),
            shape: SchemaShape::Unknown,
        });
    stack.remove(name);
    value
}

fn schema_kind_projection(
    kind: &SchemaKind,
    api: &OpenAPI,
    stack: &mut BTreeSet<String>,
) -> ProjectedSchema {
    match kind {
        SchemaKind::Type(Type::String(_)) => primitive_projection(SchemaPrimitive::String),
        SchemaKind::Type(Type::Number(_)) => primitive_projection(SchemaPrimitive::Number),
        SchemaKind::Type(Type::Integer(_)) => primitive_projection(SchemaPrimitive::Integer),
        SchemaKind::Type(Type::Boolean(_)) => primitive_projection(SchemaPrimitive::Boolean),
        SchemaKind::Type(Type::Array(array)) => {
            let mut value = json!({ "type": "array" });
            let items = array.items.as_ref().map(|items| {
                let projected = boxed_schema_projection_with_stack(items, api, stack);
                value["items"] = projected.json;
                Box::new(projected.shape)
            });
            ProjectedSchema {
                json: value,
                shape: SchemaShape::Array { items },
            }
        }
        SchemaKind::Type(Type::Object(object)) => {
            let mut json_properties = Map::new();
            let mut projected_properties = BTreeMap::new();
            for (name, schema) in &object.properties {
                let projected = boxed_schema_projection_with_stack(schema, api, stack);
                json_properties.insert(name.clone(), projected.json.clone());
                projected_properties.insert(name.clone(), projected);
            }
            ProjectedSchema {
                json: json!({
                    "type": "object",
                    "properties": json_properties,
                    "required": object.required,
                }),
                shape: SchemaShape::Object {
                    properties: projected_properties,
                    required: object.required.clone(),
                    flattenable: true,
                },
            }
        }
        _ => ProjectedSchema {
            json: json!({}),
            shape: SchemaShape::Unknown,
        },
    }
}

fn primitive_projection(primitive: SchemaPrimitive) -> ProjectedSchema {
    ProjectedSchema {
        json: json!({ "type": primitive.as_json_type() }),
        shape: SchemaShape::Primitive(primitive),
    }
}
