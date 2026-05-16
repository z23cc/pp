use openapiv3::{OpenAPI, ReferenceOr, Schema, SchemaKind, Type};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub(super) fn schema_json(schema: &ReferenceOr<Schema>, api: &OpenAPI) -> Value {
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
