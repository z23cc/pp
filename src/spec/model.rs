use serde_json::Value;

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

    pub(crate) fn root_security_requirements(&self) -> Option<Vec<Vec<String>>> {
        super::operation::security_requirement_names(self.doc.get("security")?)
    }

    #[cfg(test)]
    pub(crate) fn retain_paths_for_tests(&mut self, mut keep: impl FnMut(&str) -> bool) {
        if let Some(paths) = self.doc.get_mut("paths").and_then(Value::as_object_mut) {
            paths.retain(|path, _| keep(path));
        }
    }
}
