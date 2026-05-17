#![allow(dead_code)]

use serde::Serialize;

pub(crate) const SUPPORT_MATRIX_ID: &str = "pp.strict-openapi-support.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SupportStatus {
    Supported,
    Unsupported,
    Required,
    Conditional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct SupportFeature {
    pub id: &'static str,
    pub status: SupportStatus,
    pub summary: &'static str,
    pub diagnostic_codes: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SupportMatrixPayload {
    pub matrix_id: &'static str,
    pub features: &'static [SupportFeature],
    pub diagnostic_codes: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SupportDiagnosticPayload {
    pub matrix_id: &'static str,
    pub diagnostic_code: &'static str,
    pub features: Vec<SupportFeature>,
}

pub(crate) fn support_payload() -> SupportMatrixPayload {
    SupportMatrixPayload {
        matrix_id: SUPPORT_MATRIX_ID,
        features: SUPPORT_FEATURES,
        diagnostic_codes: ALL_DIAGNOSTIC_CODES,
    }
}

pub(crate) fn feature_by_id(id: &str) -> Option<&'static SupportFeature> {
    SUPPORT_FEATURES.iter().find(|feature| feature.id == id)
}

pub(crate) fn features_for_diagnostic(code: &str) -> Option<SupportDiagnosticPayload> {
    let diagnostic_code = ALL_DIAGNOSTIC_CODES
        .iter()
        .copied()
        .find(|candidate| *candidate == code)?;
    let features = SUPPORT_FEATURES
        .iter()
        .copied()
        .filter(|feature| feature.diagnostic_codes.contains(&diagnostic_code))
        .collect();
    Some(SupportDiagnosticPayload {
        matrix_id: SUPPORT_MATRIX_ID,
        diagnostic_code,
        features,
    })
}

pub(crate) mod diagnostics {
    pub(crate) mod schema {
        pub(crate) const BOOLEAN_OR_NON_OBJECT_SCHEMA: &str = "schema.boolean_or_non_object";
        pub(crate) const REF_SIBLINGS: &str = "schema.ref_siblings";
        pub(crate) const KEYWORD_UNSUPPORTED: &str = "schema.keyword_unsupported";
        pub(crate) const TYPE_UNSUPPORTED: &str = "schema.type_unsupported";
        pub(crate) const UNRESOLVED_REFERENCE: &str = "schema.unresolved_reference";
        pub(crate) const TUPLE_ARRAY_ITEMS: &str = "schema.tuple_array_items";
        pub(crate) const INVALID_TYPE_ARRAY: &str = "schema.invalid_type_array";
        pub(crate) const UNSUPPORTED_TYPE_UNION: &str = "schema.unsupported_type_union";
        pub(crate) const INVALID_TYPE: &str = "schema.invalid_type";
        pub(crate) const MISSING_SUPPORTED_TYPE: &str = "schema.missing_supported_type";

        pub(crate) const ALL_CODES: &[&str] = &[
            BOOLEAN_OR_NON_OBJECT_SCHEMA,
            REF_SIBLINGS,
            KEYWORD_UNSUPPORTED,
            TYPE_UNSUPPORTED,
            UNRESOLVED_REFERENCE,
            TUPLE_ARRAY_ITEMS,
            INVALID_TYPE_ARRAY,
            UNSUPPORTED_TYPE_UNION,
            INVALID_TYPE,
            MISSING_SUPPORTED_TYPE,
        ];
    }

    pub(crate) mod spec {
        pub(crate) const LOAD_ERROR: &str = "spec.load_error";

        pub(crate) const ALL_CODES: &[&str] = &[LOAD_ERROR];
    }

    pub(crate) mod runtime {
        pub(crate) const BASE_URL: &str = "runtime.base_url";

        pub(crate) const ALL_CODES: &[&str] = &[BASE_URL];
    }

    pub(crate) mod model {
        pub(crate) const GENERATION_ERROR: &str = "model.generation_error";

        pub(crate) const ALL_CODES: &[&str] = &[GENERATION_ERROR];
    }

    pub(crate) mod direct_http {
        pub(crate) const UNRESOLVED_PARAMETER_REF: &str = "direct_http.unresolved_parameter_ref";
        pub(crate) const PARAMETER_NAME_MISSING: &str = "direct_http.parameter_name_missing";
        pub(crate) const PARAMETER_LOCATION_MISSING: &str =
            "direct_http.parameter_location_missing";
        pub(crate) const PARAMETER_LOCATION_UNSUPPORTED: &str =
            "direct_http.parameter_location_unsupported";
        pub(crate) const PARAMETER_REQUIRED_NULLABLE: &str =
            "direct_http.parameter_required_nullable";
        pub(crate) const PARAMETER_SCHEMA_UNSUPPORTED: &str =
            "direct_http.parameter_schema_unsupported";
        pub(crate) const PARAMETER_PRIMITIVE_TYPE_MISSING: &str =
            "direct_http.parameter_primitive_type_missing";
        pub(crate) const QUERY_ARRAY_ITEM_NULLABLE: &str = "direct_http.query_array_item_nullable";
        pub(crate) const PARAMETER_ARRAY_NON_PRIMITIVE: &str =
            "direct_http.parameter_array_non_primitive";
        pub(crate) const PATH_ARRAY_UNSUPPORTED: &str = "direct_http.path_array_unsupported";
        pub(crate) const PARAMETER_TYPE_UNSUPPORTED: &str =
            "direct_http.parameter_type_unsupported";
        pub(crate) const PARAMETER_CONTENT_ENCODING: &str =
            "direct_http.parameter_content_encoding";
        pub(crate) const PARAMETER_SCHEMA_MISSING: &str = "direct_http.parameter_schema_missing";
        pub(crate) const QUERY_STYLE_NON_FORM: &str = "direct_http.query_style_non_form";
        pub(crate) const QUERY_ARRAY_NON_EXPLODED: &str = "direct_http.query_array_non_exploded";
        pub(crate) const PATH_STYLE_NON_SIMPLE: &str = "direct_http.path_style_non_simple";
        pub(crate) const UNRESOLVED_REQUEST_BODY_REF: &str =
            "direct_http.unresolved_request_body_ref";
        pub(crate) const REQUEST_BODY_JSON_MISSING: &str = "direct_http.request_body_json_missing";
        pub(crate) const REQUEST_BODY_NON_JSON: &str = "direct_http.request_body_non_json";
        pub(crate) const REQUEST_BODY_SCHEMA_MISSING: &str =
            "direct_http.request_body_schema_missing";
        pub(crate) const REQUEST_BODY_FIELD_SCHEMA_UNSUPPORTED: &str =
            "direct_http.request_body_field_schema_unsupported";
        pub(crate) const REQUEST_BODY_SCHEMA_UNSUPPORTED: &str =
            "direct_http.request_body_schema_unsupported";
        pub(crate) const REQUEST_BODY_FIELD_COLLISION: &str =
            "direct_http.request_body_field_collision";

        pub(crate) const ALL_CODES: &[&str] = &[
            UNRESOLVED_PARAMETER_REF,
            PARAMETER_NAME_MISSING,
            PARAMETER_LOCATION_MISSING,
            PARAMETER_LOCATION_UNSUPPORTED,
            PARAMETER_REQUIRED_NULLABLE,
            PARAMETER_SCHEMA_UNSUPPORTED,
            PARAMETER_PRIMITIVE_TYPE_MISSING,
            QUERY_ARRAY_ITEM_NULLABLE,
            PARAMETER_ARRAY_NON_PRIMITIVE,
            PATH_ARRAY_UNSUPPORTED,
            PARAMETER_TYPE_UNSUPPORTED,
            PARAMETER_CONTENT_ENCODING,
            PARAMETER_SCHEMA_MISSING,
            QUERY_STYLE_NON_FORM,
            QUERY_ARRAY_NON_EXPLODED,
            PATH_STYLE_NON_SIMPLE,
            UNRESOLVED_REQUEST_BODY_REF,
            REQUEST_BODY_JSON_MISSING,
            REQUEST_BODY_NON_JSON,
            REQUEST_BODY_SCHEMA_MISSING,
            REQUEST_BODY_FIELD_SCHEMA_UNSUPPORTED,
            REQUEST_BODY_SCHEMA_UNSUPPORTED,
            REQUEST_BODY_FIELD_COLLISION,
        ];
    }
}

pub(crate) const ALL_DIAGNOSTIC_CODES: &[&str] = &[
    diagnostics::spec::LOAD_ERROR,
    diagnostics::runtime::BASE_URL,
    diagnostics::model::GENERATION_ERROR,
    diagnostics::schema::BOOLEAN_OR_NON_OBJECT_SCHEMA,
    diagnostics::schema::REF_SIBLINGS,
    diagnostics::schema::KEYWORD_UNSUPPORTED,
    diagnostics::schema::TYPE_UNSUPPORTED,
    diagnostics::schema::UNRESOLVED_REFERENCE,
    diagnostics::schema::TUPLE_ARRAY_ITEMS,
    diagnostics::schema::INVALID_TYPE_ARRAY,
    diagnostics::schema::UNSUPPORTED_TYPE_UNION,
    diagnostics::schema::INVALID_TYPE,
    diagnostics::schema::MISSING_SUPPORTED_TYPE,
    diagnostics::direct_http::UNRESOLVED_PARAMETER_REF,
    diagnostics::direct_http::PARAMETER_NAME_MISSING,
    diagnostics::direct_http::PARAMETER_LOCATION_MISSING,
    diagnostics::direct_http::PARAMETER_LOCATION_UNSUPPORTED,
    diagnostics::direct_http::PARAMETER_REQUIRED_NULLABLE,
    diagnostics::direct_http::PARAMETER_SCHEMA_UNSUPPORTED,
    diagnostics::direct_http::PARAMETER_PRIMITIVE_TYPE_MISSING,
    diagnostics::direct_http::QUERY_ARRAY_ITEM_NULLABLE,
    diagnostics::direct_http::PARAMETER_ARRAY_NON_PRIMITIVE,
    diagnostics::direct_http::PATH_ARRAY_UNSUPPORTED,
    diagnostics::direct_http::PARAMETER_TYPE_UNSUPPORTED,
    diagnostics::direct_http::PARAMETER_CONTENT_ENCODING,
    diagnostics::direct_http::PARAMETER_SCHEMA_MISSING,
    diagnostics::direct_http::QUERY_STYLE_NON_FORM,
    diagnostics::direct_http::QUERY_ARRAY_NON_EXPLODED,
    diagnostics::direct_http::PATH_STYLE_NON_SIMPLE,
    diagnostics::direct_http::UNRESOLVED_REQUEST_BODY_REF,
    diagnostics::direct_http::REQUEST_BODY_JSON_MISSING,
    diagnostics::direct_http::REQUEST_BODY_NON_JSON,
    diagnostics::direct_http::REQUEST_BODY_SCHEMA_MISSING,
    diagnostics::direct_http::REQUEST_BODY_FIELD_SCHEMA_UNSUPPORTED,
    diagnostics::direct_http::REQUEST_BODY_SCHEMA_UNSUPPORTED,
    diagnostics::direct_http::REQUEST_BODY_FIELD_COLLISION,
];

pub(crate) const SUPPORT_FEATURES: &[SupportFeature] = &[
    SupportFeature {
        id: "spec.load",
        status: SupportStatus::Required,
        summary: "Input specs must be readable, parseable OpenAPI documents before check or generation can continue.",
        diagnostic_codes: &[diagnostics::spec::LOAD_ERROR],
    },
    SupportFeature {
        id: "openapi.3_0.strict_subset",
        status: SupportStatus::Supported,
        summary: "OpenAPI 3.0 documents are parsed strictly without repair or fallback generation.",
        diagnostic_codes: &[],
    },
    SupportFeature {
        id: "openapi.3_1.safe_subset",
        status: SupportStatus::Conditional,
        summary: "OpenAPI 3.1 is limited to primitive params, JSON bodies, refs, and nullable [T, null] unions.",
        diagnostic_codes: &[
            diagnostics::schema::KEYWORD_UNSUPPORTED,
            diagnostics::schema::UNSUPPORTED_TYPE_UNION,
        ],
    },
    SupportFeature {
        id: "operation.operation_id",
        status: SupportStatus::Required,
        summary: "Every generated operation must declare a stable explicit operationId.",
        diagnostic_codes: &[],
    },
    SupportFeature {
        id: "runtime.base_url",
        status: SupportStatus::Required,
        summary: "Generated workspaces require an absolute server URL or --base-url override.",
        diagnostic_codes: &[diagnostics::runtime::BASE_URL],
    },
    SupportFeature {
        id: "model.generation",
        status: SupportStatus::Required,
        summary: "The selected operation set must be modelable by pp's native direct HTTP generator.",
        diagnostic_codes: &[diagnostics::model::GENERATION_ERROR],
    },
    SupportFeature {
        id: "parameters.path_query_primitives",
        status: SupportStatus::Supported,
        summary: "Path and query parameters support string, integer, number, and boolean schemas.",
        diagnostic_codes: &[
            diagnostics::direct_http::UNRESOLVED_PARAMETER_REF,
            diagnostics::direct_http::PARAMETER_NAME_MISSING,
            diagnostics::direct_http::PARAMETER_LOCATION_MISSING,
            diagnostics::direct_http::PARAMETER_LOCATION_UNSUPPORTED,
            diagnostics::direct_http::PARAMETER_REQUIRED_NULLABLE,
            diagnostics::direct_http::PARAMETER_SCHEMA_UNSUPPORTED,
            diagnostics::direct_http::PARAMETER_PRIMITIVE_TYPE_MISSING,
            diagnostics::direct_http::PATH_ARRAY_UNSUPPORTED,
            diagnostics::direct_http::PARAMETER_TYPE_UNSUPPORTED,
            diagnostics::direct_http::PARAMETER_CONTENT_ENCODING,
            diagnostics::direct_http::PARAMETER_SCHEMA_MISSING,
            diagnostics::direct_http::QUERY_STYLE_NON_FORM,
            diagnostics::direct_http::PATH_STYLE_NON_SIMPLE,
        ],
    },
    SupportFeature {
        id: "parameters.query_arrays_exploded_primitives",
        status: SupportStatus::Supported,
        summary: "Query arrays are supported only as exploded arrays of primitive non-null items.",
        diagnostic_codes: &[
            diagnostics::direct_http::QUERY_ARRAY_NON_EXPLODED,
            diagnostics::direct_http::QUERY_ARRAY_ITEM_NULLABLE,
            diagnostics::direct_http::PARAMETER_ARRAY_NON_PRIMITIVE,
        ],
    },
    SupportFeature {
        id: "request_bodies.json",
        status: SupportStatus::Supported,
        summary: "JSON request bodies are supported as flattened object fields or one whole body argument.",
        diagnostic_codes: &[
            diagnostics::direct_http::REQUEST_BODY_JSON_MISSING,
            diagnostics::direct_http::REQUEST_BODY_NON_JSON,
            diagnostics::direct_http::UNRESOLVED_REQUEST_BODY_REF,
            diagnostics::direct_http::REQUEST_BODY_SCHEMA_MISSING,
            diagnostics::direct_http::REQUEST_BODY_FIELD_SCHEMA_UNSUPPORTED,
            diagnostics::direct_http::REQUEST_BODY_SCHEMA_UNSUPPORTED,
            diagnostics::direct_http::REQUEST_BODY_FIELD_COLLISION,
        ],
    },
    SupportFeature {
        id: "schemas.refs_nullable",
        status: SupportStatus::Supported,
        summary: "components/schemas refs, $defs refs, and nullable type [T, null] are supported in the safe subset.",
        diagnostic_codes: &[
            diagnostics::schema::BOOLEAN_OR_NON_OBJECT_SCHEMA,
            diagnostics::schema::UNRESOLVED_REFERENCE,
            diagnostics::schema::REF_SIBLINGS,
            diagnostics::schema::TYPE_UNSUPPORTED,
            diagnostics::schema::INVALID_TYPE_ARRAY,
            diagnostics::schema::INVALID_TYPE,
            diagnostics::schema::MISSING_SUPPORTED_TYPE,
        ],
    },
    SupportFeature {
        id: "auth.supported_schemes",
        status: SupportStatus::Supported,
        summary: "Generated auth supports none, HTTP bearer, header apiKey, and HTTP basic.",
        diagnostic_codes: &[],
    },
    SupportFeature {
        id: "json_schema.broad_2020_12",
        status: SupportStatus::Unsupported,
        summary: "Broad JSON Schema 2020-12 features such as composition, conditionals, tuple arrays, additionalProperties, and broad type unions are unsupported diagnostics.",
        diagnostic_codes: &[
            diagnostics::schema::KEYWORD_UNSUPPORTED,
            diagnostics::schema::TUPLE_ARRAY_ITEMS,
            diagnostics::schema::UNSUPPORTED_TYPE_UNION,
            diagnostics::schema::INVALID_TYPE,
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn support_matrix_has_stable_id_and_unique_feature_ids() {
        assert_eq!(SUPPORT_MATRIX_ID, "pp.strict-openapi-support.v1");
        let mut ids = BTreeSet::new();
        for feature in SUPPORT_FEATURES {
            assert!(
                ids.insert(feature.id),
                "duplicate feature id {}",
                feature.id
            );
            assert!(!feature.summary.is_empty());
        }
    }

    #[test]
    fn support_matrix_diagnostic_codes_are_unique_per_feature() {
        let declared: BTreeSet<_> = ALL_DIAGNOSTIC_CODES.iter().copied().collect();
        for feature in SUPPORT_FEATURES {
            let mut codes = BTreeSet::new();
            for code in feature.diagnostic_codes {
                assert!(
                    codes.insert(*code),
                    "duplicate code {code} in {}",
                    feature.id
                );
                assert!(code.contains('.'));
                assert!(
                    declared.contains(code),
                    "{} references undeclared diagnostic code {code}",
                    feature.id
                );
            }
        }
    }

    #[test]
    fn all_diagnostic_codes_are_unique_and_namespaced() {
        let mut codes = BTreeSet::new();
        for code in ALL_DIAGNOSTIC_CODES {
            assert!(codes.insert(*code), "duplicate diagnostic code {code}");
            assert!(
                code.starts_with("spec.")
                    || code.starts_with("runtime.")
                    || code.starts_with("model.")
                    || code.starts_with("schema.")
                    || code.starts_with("direct_http."),
                "diagnostic code {code} must be in an emitted namespace"
            );
        }
    }

    #[test]
    fn all_grouped_diagnostic_codes_are_in_inventory() {
        let declared: BTreeSet<_> = ALL_DIAGNOSTIC_CODES.iter().copied().collect();
        for code in diagnostics::spec::ALL_CODES
            .iter()
            .chain(diagnostics::runtime::ALL_CODES.iter())
            .chain(diagnostics::model::ALL_CODES.iter())
            .chain(diagnostics::schema::ALL_CODES.iter())
            .chain(diagnostics::direct_http::ALL_CODES.iter())
        {
            assert!(
                declared.contains(code),
                "grouped diagnostic code {code} missing from inventory"
            );
        }
    }

    #[test]
    fn support_features_cover_declared_diagnostic_codes() {
        let covered: BTreeSet<_> = SUPPORT_FEATURES
            .iter()
            .flat_map(|feature| feature.diagnostic_codes.iter().copied())
            .collect();
        for code in ALL_DIAGNOSTIC_CODES {
            assert!(
                covered.contains(code),
                "diagnostic code {code} is not attached to any support feature"
            );
        }
    }
}
