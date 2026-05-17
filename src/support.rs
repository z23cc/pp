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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DiagnosticExplanation {
    pub matrix_id: &'static str,
    pub diagnostic_code: &'static str,
    pub title: &'static str,
    pub meaning: &'static str,
    pub remediation: &'static str,
    pub severity_hint: &'static str,
    pub strict_behavior: &'static str,
    pub features: Vec<SupportFeature>,
}

#[derive(Debug, Clone, Copy)]
struct DiagnosticExplanationMetadata {
    diagnostic_code: &'static str,
    title: &'static str,
    meaning: &'static str,
    remediation: &'static str,
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

pub(crate) fn explain_diagnostic(code: &str) -> Option<DiagnosticExplanation> {
    let metadata = DIAGNOSTIC_EXPLANATIONS
        .iter()
        .copied()
        .find(|candidate| candidate.diagnostic_code == code)?;
    let feature_payload = features_for_diagnostic(metadata.diagnostic_code)?;
    Some(DiagnosticExplanation {
        matrix_id: SUPPORT_MATRIX_ID,
        diagnostic_code: metadata.diagnostic_code,
        title: metadata.title,
        meaning: metadata.meaning,
        remediation: metadata.remediation,
        severity_hint: severity_hint_for(metadata.diagnostic_code),
        strict_behavior: strict_behavior_for(metadata.diagnostic_code),
        features: feature_payload.features,
    })
}

fn severity_hint_for(code: &str) -> &'static str {
    if code.starts_with("spec.") {
        "error: pp cannot load the selected OpenAPI input until this is resolved."
    } else if code.starts_with("runtime.") {
        "error: pp cannot generate a runnable native CLI without an explicit supported runtime value."
    } else if code.starts_with("model.") {
        "error: pp cannot build its native generation model while this condition is present."
    } else if code.starts_with("schema.") {
        "error: pp cannot model the affected schema in its strict supported subset."
    } else if code.starts_with("direct_http.") {
        "error: pp cannot generate the selected native direct HTTP operation set until this operation shape is fixed or explicitly excluded."
    } else {
        "error: pp check/generate fails until this diagnostic is resolved."
    }
}

fn strict_behavior_for(code: &str) -> &'static str {
    if code.starts_with("spec.") {
        "pp stops at load/preparation time; it does not repair malformed specs, fetch missing remote data, or guess invalid selections."
    } else if code.starts_with("runtime.") {
        "pp requires an explicit absolute http(s) runtime base URL; it does not invent or normalize one."
    } else if code.starts_with("model.") {
        "pp fails the selected operation set instead of falling back to wrapper generation or silently omitting required model data."
    } else if code.starts_with("schema.") {
        "pp reports unsupported schema shapes explicitly; it does not coerce, infer, or downgrade JSON Schema semantics."
    } else if code.starts_with("direct_http.") {
        "pp reports unsupported operation shapes explicitly and does not generate fallback runtime adapters for them."
    } else {
        "pp preserves strict no-repair/no-fallback behavior for this diagnostic."
    }
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

const DIAGNOSTIC_EXPLANATIONS: &[DiagnosticExplanationMetadata] = &[
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::spec::LOAD_ERROR,
        title: "Spec could not be loaded or prepared",
        meaning: "pp could not complete the pre-model load path: reading/parsing the OpenAPI document, applying slice include/exclude filters, or selecting the requested auth scheme failed.",
        remediation: "Fix the source spec path/syntax, adjust slice filters so at least one operation matches, or pass an auth scheme that exists in components.securitySchemes; pp does not repair malformed input or guess invalid selections.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::runtime::BASE_URL,
        title: "Runtime base URL is missing or unsupported",
        meaning: "Generated native HTTP commands need an absolute http(s) base URL.",
        remediation: "Add an absolute servers[0].url to the source spec or pass --base-url with an absolute http(s) URL.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::model::GENERATION_ERROR,
        title: "Selected operations cannot be modeled",
        meaning: "The selected operation set violates pp's strict native generation contract before direct HTTP planning can finish.",
        remediation: "Fix the source operation shapes or narrow the selection with include/exclude filters; pp does not fallback to generated wrapper execution.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::schema::BOOLEAN_OR_NON_OBJECT_SCHEMA,
        title: "Schema is boolean or non-object",
        meaning: "pp expects schemas it models to be JSON objects with explicit supported keywords.",
        remediation: "Replace boolean/non-object schema declarations with explicit object schemas in the source spec.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::schema::REF_SIBLINGS,
        title: "$ref has sibling keywords",
        meaning: "A schema combines $ref with sibling keywords, which pp treats as ambiguous in the strict subset.",
        remediation: "Move sibling constraints into the referenced schema or select a simpler schema shape.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::schema::KEYWORD_UNSUPPORTED,
        title: "Unsupported schema keyword",
        meaning: "The schema uses a JSON Schema/OpenAPI keyword outside pp's supported modeling subset.",
        remediation: "Remove or simplify the keyword in the source spec, or exclude operations that require it.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::schema::TYPE_UNSUPPORTED,
        title: "Unsupported schema type",
        meaning: "The schema declares a type that pp cannot map to generated arguments or JSON body fields.",
        remediation: "Change the source schema to a supported primitive/object/array shape or exclude the affected operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::schema::UNRESOLVED_REFERENCE,
        title: "Schema reference could not be resolved",
        meaning: "A local schema reference points to a target pp cannot find in the loaded document.",
        remediation: "Fix the local $ref target in the source spec; remote fetching and repair are not performed.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::schema::TUPLE_ARRAY_ITEMS,
        title: "Tuple array items are unsupported",
        meaning: "The schema describes array positions with tuple-style items rather than one item schema.",
        remediation: "Use a homogeneous array item schema or exclude the operation that requires tuple arrays.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::schema::INVALID_TYPE_ARRAY,
        title: "Invalid schema type array",
        meaning: "A type array is not one of pp's accepted nullable [T, null] forms.",
        remediation: "Rewrite the schema type to a single supported type or a nullable [T, null] pair.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::schema::UNSUPPORTED_TYPE_UNION,
        title: "Unsupported type union",
        meaning: "The schema uses a type union broader than pp's safe nullable subset.",
        remediation: "Reduce the union to one supported type plus optional null, or exclude the affected operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::schema::INVALID_TYPE,
        title: "Invalid schema type",
        meaning: "The schema type value is missing, malformed, or not a recognized OpenAPI/JSON Schema type.",
        remediation: "Correct the type declaration in the source spec before running pp again.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::schema::MISSING_SUPPORTED_TYPE,
        title: "Missing supported schema type",
        meaning: "pp could not infer a supported type from the schema declaration.",
        remediation: "Add an explicit supported type or simplify the schema used by the selected operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::UNRESOLVED_PARAMETER_REF,
        title: "Parameter reference could not be resolved",
        meaning: "A parameter $ref used by a selected operation does not resolve locally.",
        remediation: "Fix the parameter $ref in the source spec or exclude the affected operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PARAMETER_NAME_MISSING,
        title: "Parameter name is missing",
        meaning: "A selected operation has a parameter without a name.",
        remediation: "Add a stable parameter name to the source spec or exclude the operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PARAMETER_LOCATION_MISSING,
        title: "Parameter location is missing",
        meaning: "A selected operation has a parameter without an in location.",
        remediation: "Set the parameter location to a supported value such as path or query.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PARAMETER_LOCATION_UNSUPPORTED,
        title: "Parameter location is unsupported",
        meaning: "The parameter is not in a location pp's native direct HTTP planner supports.",
        remediation: "Use supported path/query parameters or exclude the affected operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PARAMETER_REQUIRED_NULLABLE,
        title: "Required parameter is nullable",
        meaning: "A required parameter allows null, which cannot be represented as a required CLI argument safely.",
        remediation: "Make the parameter non-nullable or change the source contract before generation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PARAMETER_SCHEMA_UNSUPPORTED,
        title: "Parameter schema is unsupported",
        meaning: "The parameter schema shape is outside pp's supported parameter subset.",
        remediation: "Use primitive path/query parameters or supported exploded primitive query arrays.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PARAMETER_PRIMITIVE_TYPE_MISSING,
        title: "Parameter primitive type is missing",
        meaning: "A parameter needs an explicit primitive schema type for CLI argument generation.",
        remediation: "Add a string, integer, number, or boolean type to the parameter schema.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::QUERY_ARRAY_ITEM_NULLABLE,
        title: "Query array item is nullable",
        meaning: "pp supports query arrays only when each item is a non-null primitive.",
        remediation: "Make the array item non-nullable or exclude the affected operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PARAMETER_ARRAY_NON_PRIMITIVE,
        title: "Parameter array item is not primitive",
        meaning: "A parameter array contains object or otherwise non-primitive items.",
        remediation: "Use an exploded array of primitive items or exclude the affected operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PATH_ARRAY_UNSUPPORTED,
        title: "Path array parameter is unsupported",
        meaning: "pp does not model array-valued path parameters for native direct HTTP invocation.",
        remediation: "Use primitive path parameters or exclude the affected operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PARAMETER_TYPE_UNSUPPORTED,
        title: "Parameter type is unsupported",
        meaning: "A parameter uses a type pp cannot expose as a native CLI argument.",
        remediation: "Change the source parameter to a supported primitive shape or exclude the affected operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PARAMETER_CONTENT_ENCODING,
        title: "Parameter content encoding is unsupported",
        meaning: "The parameter uses OpenAPI content encoding instead of a simple schema.",
        remediation: "Represent the parameter with a supported schema or exclude the operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PARAMETER_SCHEMA_MISSING,
        title: "Parameter schema is missing",
        meaning: "A selected parameter has no schema pp can inspect.",
        remediation: "Add a supported parameter schema to the source spec.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::QUERY_STYLE_NON_FORM,
        title: "Query parameter style is unsupported",
        meaning: "pp supports query serialization through the form style only.",
        remediation: "Use form-style query parameters or exclude the affected operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::QUERY_ARRAY_NON_EXPLODED,
        title: "Query array is not exploded",
        meaning: "pp supports query arrays only as exploded repeated query parameters.",
        remediation: "Set explode: true for the array query parameter or exclude the operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::PATH_STYLE_NON_SIMPLE,
        title: "Path parameter style is unsupported",
        meaning: "pp supports simple path parameter serialization only.",
        remediation: "Use simple path style or exclude the affected operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::UNRESOLVED_REQUEST_BODY_REF,
        title: "Request body reference could not be resolved",
        meaning: "A requestBody $ref used by a selected operation does not resolve locally.",
        remediation: "Fix the request body $ref target in the source spec or exclude the operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::REQUEST_BODY_JSON_MISSING,
        title: "JSON request body is missing",
        meaning: "A selected operation has a request body but no application/json media type pp can model.",
        remediation: "Add an application/json request body schema or exclude the operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::REQUEST_BODY_NON_JSON,
        title: "Request body is not JSON",
        meaning: "The operation requires a non-JSON request body outside pp's native direct HTTP subset.",
        remediation: "Use a JSON request body for the source operation or exclude it from generation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::REQUEST_BODY_SCHEMA_MISSING,
        title: "Request body schema is missing",
        meaning: "The JSON request body lacks a schema pp can model.",
        remediation: "Add a supported JSON schema for the request body or exclude the operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::REQUEST_BODY_FIELD_SCHEMA_UNSUPPORTED,
        title: "Request body field schema is unsupported",
        meaning: "A flattened JSON body field uses a schema shape pp cannot expose as an argument.",
        remediation: "Simplify that field schema or model the body with supported JSON shapes.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::REQUEST_BODY_SCHEMA_UNSUPPORTED,
        title: "Request body schema is unsupported",
        meaning: "The JSON request body schema is outside pp's supported body modeling subset.",
        remediation: "Simplify the source schema, use a supported object/body shape, or exclude the operation.",
    },
    DiagnosticExplanationMetadata {
        diagnostic_code: diagnostics::direct_http::REQUEST_BODY_FIELD_COLLISION,
        title: "Request body field collides with another argument",
        meaning: "A generated JSON body field argument would conflict with another CLI argument name.",
        remediation: "Rename fields or parameters in the source spec, or exclude the affected operation.",
    },
];

pub(crate) const SUPPORT_FEATURES: &[SupportFeature] = &[
    SupportFeature {
        id: "spec.load",
        status: SupportStatus::Required,
        summary: "Input specs must be readable, parseable, sliceable, and have a valid auth selection before check or generation can continue.",
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

    #[test]
    fn every_diagnostic_code_has_an_explanation() {
        let explanation_codes: BTreeSet<_> = DIAGNOSTIC_EXPLANATIONS
            .iter()
            .map(|explanation| explanation.diagnostic_code)
            .collect();
        for code in ALL_DIAGNOSTIC_CODES {
            let explanation = explain_diagnostic(code)
                .unwrap_or_else(|| panic!("diagnostic code {code} has no explanation"));
            assert!(explanation_codes.contains(code));
            assert_eq!(explanation.matrix_id, SUPPORT_MATRIX_ID);
            assert_eq!(explanation.diagnostic_code, *code);
            assert!(!explanation.title.is_empty());
            assert!(!explanation.meaning.is_empty());
            assert!(!explanation.remediation.is_empty());
            assert!(!explanation.severity_hint.is_empty());
            assert!(!explanation.strict_behavior.is_empty());
            assert!(!explanation.features.is_empty());
        }
    }

    #[test]
    fn unknown_diagnostic_has_no_explanation() {
        assert!(explain_diagnostic("not.real").is_none());
    }
}
