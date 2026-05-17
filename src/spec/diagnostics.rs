use std::fmt;

use crate::support::diagnostics::schema as schema_codes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnsupportedSchemaDiagnostic {
    feature: SchemaFeature,
    pointer: String,
}

impl UnsupportedSchemaDiagnostic {
    pub(crate) fn new(feature: SchemaFeature, pointer: impl Into<String>) -> Self {
        Self {
            feature,
            pointer: pointer.into(),
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        self.feature.code()
    }
}

impl fmt::Display for UnsupportedSchemaDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if matches!(self.feature, SchemaFeature::InvalidTypeArray) {
            return write!(
                formatter,
                "{} at {} at {}",
                self.feature, self.pointer, self.pointer
            );
        }
        write!(formatter, "{} at {}", self.feature, self.pointer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SchemaFeature {
    BooleanOrNonObjectSchema,
    RefSiblings,
    SchemaKeyword(String),
    UnsupportedJsonSchemaType(String),
    UnresolvedReference(String),
    TupleArrayItems,
    InvalidTypeArray,
    UnsupportedTypeUnion,
    InvalidType,
    MissingSupportedType,
}

impl SchemaFeature {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::BooleanOrNonObjectSchema => schema_codes::BOOLEAN_OR_NON_OBJECT_SCHEMA,
            Self::RefSiblings => schema_codes::REF_SIBLINGS,
            Self::SchemaKeyword(_) => schema_codes::KEYWORD_UNSUPPORTED,
            Self::UnsupportedJsonSchemaType(_) => schema_codes::TYPE_UNSUPPORTED,
            Self::UnresolvedReference(_) => schema_codes::UNRESOLVED_REFERENCE,
            Self::TupleArrayItems => schema_codes::TUPLE_ARRAY_ITEMS,
            Self::InvalidTypeArray => schema_codes::INVALID_TYPE_ARRAY,
            Self::UnsupportedTypeUnion => schema_codes::UNSUPPORTED_TYPE_UNION,
            Self::InvalidType => schema_codes::INVALID_TYPE,
            Self::MissingSupportedType => schema_codes::MISSING_SUPPORTED_TYPE,
        }
    }
}

impl fmt::Display for SchemaFeature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BooleanOrNonObjectSchema => {
                write!(formatter, "boolean/non-object schemas are not supported")
            }
            Self::RefSiblings => write!(formatter, "schemas with $ref siblings are not supported"),
            Self::SchemaKeyword(feature) => {
                write!(formatter, "unsupported JSON Schema feature '{feature}'")
            }
            Self::UnsupportedJsonSchemaType(schema_type) => {
                write!(formatter, "unsupported JSON Schema type '{schema_type}'")
            }
            Self::UnresolvedReference(reference) => {
                write!(formatter, "unresolved schema reference '{reference}'")
            }
            Self::TupleArrayItems => write!(formatter, "unsupported JSON Schema tuple array items"),
            Self::InvalidTypeArray => write!(formatter, "invalid JSON Schema type array"),
            Self::UnsupportedTypeUnion => write!(
                formatter,
                "unsupported JSON Schema type union; only [T, null] is supported"
            ),
            Self::InvalidType => write!(formatter, "invalid JSON Schema type"),
            Self::MissingSupportedType => write!(
                formatter,
                "schema without primitive schema type or supported object/array type"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_type_array_preserves_legacy_duplicate_pointer_message() {
        let diagnostic = UnsupportedSchemaDiagnostic::new(
            SchemaFeature::InvalidTypeArray,
            "/components/schemas/Bad/type",
        );

        assert_eq!(
            diagnostic.to_string(),
            "invalid JSON Schema type array at /components/schemas/Bad/type at /components/schemas/Bad/type"
        );
        assert_eq!(diagnostic.code(), schema_codes::INVALID_TYPE_ARRAY);
    }

    #[test]
    fn schema_features_expose_stable_codes() {
        assert_eq!(
            SchemaFeature::SchemaKeyword("oneOf".to_string()).code(),
            schema_codes::KEYWORD_UNSUPPORTED
        );
        assert_eq!(
            SchemaFeature::UnsupportedTypeUnion.code(),
            schema_codes::UNSUPPORTED_TYPE_UNION
        );
    }
}
