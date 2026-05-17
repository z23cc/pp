use serde::Serialize;

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
