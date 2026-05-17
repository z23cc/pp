use serde_json::Value;

pub(crate) fn resolve_local_ref<'a>(doc: &'a Value, reference: &str) -> Option<&'a Value> {
    let pointer = reference.strip_prefix('#')?;
    doc.pointer(pointer)
}

pub(crate) fn encode_json_pointer_segment(input: &str) -> String {
    input.replace('~', "~0").replace('/', "~1")
}

pub(crate) fn decode_json_pointer_segment(input: &str) -> String {
    input.replace("~1", "/").replace("~0", "~")
}
