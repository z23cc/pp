use serde_json::{Map, Value};

use super::json_pointer::encode_json_pointer_segment;
use super::schema::PpSchemaRef;

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

pub(crate) fn security_requirement_names(value: &Value) -> Option<Vec<Vec<String>>> {
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(Value::as_object)
            .map(|requirement| requirement.keys().cloned().collect())
            .collect(),
    )
}
