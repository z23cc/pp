//! In-process driver around the `progenitor` library.

use crate::backend::{
    ApiCrateOutput, BackendDiagnostic, SourceTransformDiagnostic, SourceTransformPurpose,
    SourceTransformRequiredness, SourceTransformStatus,
};
use anyhow::{anyhow, Context, Result};
use openapiv3::OpenAPI;
use progenitor::{GenerationSettings, Generator, InterfaceStyle};
use quote::{format_ident, quote};
use regex::Regex;
use std::fs;
use std::path::Path;

const TRANSFORM_STRING_VEC_VALUE_PARSER: &str = "string_vec_value_parser";
const TRANSFORM_UNEXPECTED_RESPONSE_BODY: &str = "unexpected_response_body";
const TRANSFORM_COMPLEX_CLAP_VALUE_PARSERS: &str = "complex_clap_value_parsers";
const PROGENITOR_SOURCE_VERSION_ASSUMPTION: &str = "progenitor 0.14 generated Rust source shape";

#[derive(Debug, Clone, Copy)]
struct SourceTransformMetadata {
    name: &'static str,
    purpose: SourceTransformPurpose,
    required: SourceTransformRequiredness,
    precondition: &'static str,
    postcondition: &'static str,
    upstream_assumption: &'static str,
    upstream_version: &'static str,
}

const STRING_VEC_VALUE_PARSER_METADATA: SourceTransformMetadata = SourceTransformMetadata {
    name: TRANSFORM_STRING_VEC_VALUE_PARSER,
    purpose: SourceTransformPurpose::ClapParserCompatibility,
    required: SourceTransformRequiredness::Conditional,
    precondition: "generated CLI contains clap value_parser! calls for Vec<String>",
    postcondition: "generated source contains no Vec<String> clap value_parser! shape requiring pp compatibility",
    upstream_assumption:
        "the generated parser shape does not accept comma-separated CLI values for string arrays",
    upstream_version: PROGENITOR_SOURCE_VERSION_ASSUMPTION,
};

const UNEXPECTED_RESPONSE_BODY_METADATA: SourceTransformMetadata = SourceTransformMetadata {
    name: TRANSFORM_UNEXPECTED_RESPONSE_BODY,
    purpose: SourceTransformPurpose::ErrorDiagnostics,
    required: SourceTransformRequiredness::Required,
    precondition: "generated client contains a fallback UnexpectedResponse arm",
    postcondition:
        "fallback UnexpectedResponse path captures status, url, headers, and response body text",
    upstream_assumption:
        "the fallback error preserves response status but omits readable body context",
    upstream_version: PROGENITOR_SOURCE_VERSION_ASSUMPTION,
};

const COMPLEX_CLAP_VALUE_PARSERS_METADATA: SourceTransformMetadata = SourceTransformMetadata {
    name: TRANSFORM_COMPLEX_CLAP_VALUE_PARSERS,
    purpose: SourceTransformPurpose::ClapParserCompatibility,
    required: SourceTransformRequiredness::Conditional,
    precondition: "generated CLI contains clap value_parser! calls for generated schema types",
    postcondition:
        "generated source contains no clap value_parser! calls for generated schema types",
    upstream_assumption:
        "generated schema types require serde_json parsing rather than clap's default typed parser",
    upstream_version: PROGENITOR_SOURCE_VERSION_ASSUMPTION,
};

const STRING_VEC_VALUE_PARSER_INLINE: &str =
    "::clap::value_parser!(::std::vec::Vec<::std::string::String>)";
const STRING_VEC_VALUE_PARSER_PRETTY: &str =
    "::clap::value_parser!(\n                                ::std::vec::Vec < ::std::string::String >\n                            )";
const STRING_VEC_VALUE_PARSER_REPLACEMENT: &str = "::clap::builder::ValueParser::new(|s: &str| -> Result<::std::vec::Vec<::std::string::String>, ::std::string::String> { Ok(s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()) })";

const UNEXPECTED_RESPONSE_NEEDLE: &str = "_ => Err(Error::UnexpectedResponse(response)),";
const UNEXPECTED_RESPONSE_REPLACEMENT: &str = r#"_ => {
                let status = response.status();
                let url = response.url().clone();
                let headers = response.headers().clone();
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|err| format!("<failed to read response body: {err}>"));
                Err(Error::Custom(format!(
                    "Unexpected Response: status={status}, url={url}, headers={headers:?}, body={body}"
                )))
            }"#;

/// Generate a complete API crate containing progenitor's client and CLI tokens.
pub fn generate(api: &OpenAPI, out_dir: &Path, crate_name: &str) -> Result<ApiCrateOutput> {
    let mut settings = GenerationSettings::default();
    settings
        .with_interface(InterfaceStyle::Builder)
        .with_derive("schemars::JsonSchema")
        .with_cli_bounds("schemars::JsonSchema")
        .with_cli_bounds("serde::Serialize")
        .with_cli_bounds("std::fmt::Debug");
    let mut generator = Generator::new(&settings);
    let client_tokens = generator.generate_tokens(api)?;
    let crate_ident = format_ident!("{}", crate_name.replace('-', "_"));
    let cli_tokens = generator.cli(api, &crate_ident.to_string())?;

    let file_tokens = quote! {
        extern crate self as #crate_ident;

        #client_tokens

        pub mod cli {
            #cli_tokens
        }
    };
    let syntax = syn::parse2(file_tokens)?;
    let formatted = prettyplease::unparse(&syntax);
    let transformed = apply_generated_source_transforms(&formatted)?;

    fs::create_dir_all(out_dir.join("src"))
        .with_context(|| format!("failed to create API src dir: {}", out_dir.display()))?;
    fs::write(out_dir.join("src/lib.rs"), transformed.source)
        .with_context(|| format!("failed to write {}", out_dir.join("src/lib.rs").display()))?;

    Ok(ApiCrateOutput {
        diagnostics: transformed.diagnostics,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedSourceTransformOutput {
    source: String,
    diagnostics: Vec<BackendDiagnostic>,
}

fn apply_generated_source_transforms(source: &str) -> Result<GeneratedSourceTransformOutput> {
    let mut source = source.to_string();
    let mut diagnostics = Vec::new();

    let (next, diagnostic) = apply_source_transform(
        STRING_VEC_VALUE_PARSER_METADATA,
        &source,
        patch_string_vec_value_parser_with_count,
        string_vec_value_parser_postcondition_met,
    )?;
    diagnostics.push(diagnostic);
    source = next;

    let (next, diagnostic) = apply_source_transform_with_required_drift_detector(
        UNEXPECTED_RESPONSE_BODY_METADATA,
        &source,
        patch_unexpected_response_body_with_count,
        unexpected_response_body_postcondition_met,
        unexpected_response_body_residual_present,
    )?;
    diagnostics.push(diagnostic);
    source = next;

    let (next, diagnostic) = apply_source_transform(
        COMPLEX_CLAP_VALUE_PARSERS_METADATA,
        &source,
        patch_complex_clap_value_parsers_with_count,
        complex_clap_value_parsers_postcondition_met,
    )?;
    diagnostics.push(diagnostic);

    Ok(GeneratedSourceTransformOutput {
        source: next,
        diagnostics,
    })
}

fn apply_source_transform(
    metadata: SourceTransformMetadata,
    before: &str,
    patch: fn(&str) -> (String, usize),
    postcondition_met: fn(&str) -> bool,
) -> Result<(String, BackendDiagnostic)> {
    apply_source_transform_with_required_drift_detector(
        metadata,
        before,
        patch,
        postcondition_met,
        |_| true,
    )
}

fn apply_source_transform_with_required_drift_detector(
    metadata: SourceTransformMetadata,
    before: &str,
    patch: fn(&str) -> (String, usize),
    postcondition_met: fn(&str) -> bool,
    required_drift_present: fn(&str) -> bool,
) -> Result<(String, BackendDiagnostic)> {
    let (after, replacement_count) = patch(before);
    let status = if replacement_count > 0 {
        SourceTransformStatus::Applied
    } else {
        match metadata.required {
            SourceTransformRequiredness::Required if required_drift_present(before) => {
                return Err(anyhow!(
                    "required generated source transform '{}' did not match its precondition: {}; upstream drift from {} must be handled explicitly",
                    metadata.name,
                    metadata.precondition,
                    metadata.upstream_version
                ));
            }
            SourceTransformRequiredness::Required => SourceTransformStatus::NotApplicable,
            SourceTransformRequiredness::Conditional => SourceTransformStatus::VerifiedNotNeeded,
        }
    };

    if !postcondition_met(&after) {
        return Err(anyhow!(
            "generated source transform '{}' failed postcondition: {}",
            metadata.name,
            metadata.postcondition
        ));
    }

    Ok((
        after.clone(),
        source_transform_diagnostic(metadata, before, &after, replacement_count, status),
    ))
}

fn source_transform_diagnostic(
    metadata: SourceTransformMetadata,
    before: &str,
    after: &str,
    replacement_count: usize,
    status: SourceTransformStatus,
) -> BackendDiagnostic {
    BackendDiagnostic::SourceTransform(SourceTransformDiagnostic {
        name: metadata.name,
        changed: before != after,
        replacement_count,
        purpose: metadata.purpose,
        required: metadata.required,
        status,
        precondition: metadata.precondition,
        postcondition: metadata.postcondition,
        upstream_assumption: metadata.upstream_assumption,
        upstream_version: metadata.upstream_version,
    })
}

#[cfg(test)]
fn patch_string_vec_value_parser(source: &str) -> String {
    patch_string_vec_value_parser_with_count(source).0
}

fn patch_string_vec_value_parser_with_count(source: &str) -> (String, usize) {
    let replacement_count = source.matches(STRING_VEC_VALUE_PARSER_INLINE).count()
        + source.matches(STRING_VEC_VALUE_PARSER_PRETTY).count();
    let patched = source
        .replace(
            STRING_VEC_VALUE_PARSER_INLINE,
            STRING_VEC_VALUE_PARSER_REPLACEMENT,
        )
        .replace(
            STRING_VEC_VALUE_PARSER_PRETTY,
            STRING_VEC_VALUE_PARSER_REPLACEMENT,
        );
    (patched, replacement_count)
}

fn string_vec_value_parser_postcondition_met(source: &str) -> bool {
    let residual_re = Regex::new(
        r"::clap::value_parser!\(\s*::std::vec::Vec\s*<\s*::std::string::String\s*>\s*\)",
    )
    .expect("valid string vec value parser regex");
    !residual_re.is_match(source)
}

#[cfg(test)]
fn patch_unexpected_response_body(source: &str) -> String {
    patch_unexpected_response_body_with_count(source).0
}

fn patch_unexpected_response_body_with_count(source: &str) -> (String, usize) {
    let replacement_count = source.matches(UNEXPECTED_RESPONSE_NEEDLE).count();
    let patched = source.replace(UNEXPECTED_RESPONSE_NEEDLE, UNEXPECTED_RESPONSE_REPLACEMENT);
    (patched, replacement_count)
}

fn unexpected_response_body_residual_present(source: &str) -> bool {
    let residual_re = Regex::new(r"Error\s*::\s*UnexpectedResponse\s*\(")
        .expect("valid unexpected response residual regex");
    residual_re.is_match(source)
}

fn unexpected_response_body_postcondition_met(source: &str) -> bool {
    if unexpected_response_body_residual_present(source) {
        return false;
    }
    if !source.contains(
        "Unexpected Response: status={status}, url={url}, headers={headers:?}, body={body}",
    ) {
        return true;
    }
    source.contains("Error::Custom") && source.contains(".text()")
}

#[cfg(test)]
fn patch_complex_clap_value_parsers(source: &str) -> String {
    patch_complex_clap_value_parsers_with_count(source).0
}

fn patch_complex_clap_value_parsers_with_count(source: &str) -> (String, usize) {
    let vec_re = Regex::new(
        r"::clap::value_parser!\(\s*::std::vec::Vec\s*<\s*types::([A-Za-z][A-Za-z0-9_]*)\s*>\s*\)",
    )
    .expect("valid vec value parser regex");
    let vec_count = vec_re.find_iter(source).count();
    let source = vec_re.replace_all(source, |caps: &regex::Captures<'_>| {
        complex_vec_value_parser(&caps[1])
    });

    let single_re = Regex::new(r"::clap::value_parser!\(\s*types::([A-Za-z][A-Za-z0-9_]*)\s*\)")
        .expect("valid single value parser regex");
    let single_count = single_re.find_iter(&source).count();
    let patched = single_re
        .replace_all(&source, |caps: &regex::Captures<'_>| {
            complex_single_value_parser(&caps[1])
        })
        .into_owned();
    (patched, vec_count + single_count)
}

fn complex_single_value_parser(typ: &str) -> String {
    format!(
        "::clap::builder::ValueParser::new(|s: &str| -> Result<types::{typ}, ::std::string::String> {{ ::serde_json::from_value::<types::{typ}>(::serde_json::Value::String(s.to_string())).or_else(|_| ::serde_json::from_str::<types::{typ}>(s)).map_err(|err| err.to_string()) }})"
    )
}

fn complex_vec_value_parser(typ: &str) -> String {
    format!(
        "::clap::builder::ValueParser::new(|s: &str| -> Result<::std::vec::Vec<types::{typ}>, ::std::string::String> {{ if let Ok(values) = ::serde_json::from_str::<::std::vec::Vec<types::{typ}>>(s) {{ return Ok(values); }} s.split(',').map(|part| {{ let part = part.trim(); ::serde_json::from_value::<types::{typ}>(::serde_json::Value::String(part.to_string())).or_else(|_| ::serde_json::from_str::<types::{typ}>(part)).map_err(|err| err.to_string()) }}).collect() }})"
    )
}

fn complex_clap_value_parsers_postcondition_met(source: &str) -> bool {
    let vec_re = Regex::new(
        r"::clap::value_parser!\(\s*::std::vec::Vec\s*<\s*types::([A-Za-z][A-Za-z0-9_]*)\s*>\s*\)",
    )
    .expect("valid vec value parser regex");
    let single_re = Regex::new(r"::clap::value_parser!\(\s*types::([A-Za-z][A-Za-z0-9_]*)\s*\)")
        .expect("valid single value parser regex");
    !vec_re.is_match(source) && !single_re.is_match(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_transform_names_are_stable() {
        assert_eq!(TRANSFORM_STRING_VEC_VALUE_PARSER, "string_vec_value_parser");
        assert_eq!(
            TRANSFORM_UNEXPECTED_RESPONSE_BODY,
            "unexpected_response_body"
        );
        assert_eq!(
            TRANSFORM_COMPLEX_CLAP_VALUE_PARSERS,
            "complex_clap_value_parsers"
        );
    }

    #[test]
    fn source_transform_metadata_describes_semantic_intent() {
        assert_eq!(
            STRING_VEC_VALUE_PARSER_METADATA.purpose,
            SourceTransformPurpose::ClapParserCompatibility
        );
        assert_eq!(
            UNEXPECTED_RESPONSE_BODY_METADATA.purpose,
            SourceTransformPurpose::ErrorDiagnostics
        );
        assert_eq!(
            COMPLEX_CLAP_VALUE_PARSERS_METADATA.purpose,
            SourceTransformPurpose::ClapParserCompatibility
        );

        for metadata in [
            STRING_VEC_VALUE_PARSER_METADATA,
            UNEXPECTED_RESPONSE_BODY_METADATA,
            COMPLEX_CLAP_VALUE_PARSERS_METADATA,
        ] {
            assert!(!metadata.precondition.is_empty());
            assert!(!metadata.postcondition.is_empty());
            assert!(!metadata.upstream_assumption.is_empty());
            assert_eq!(
                metadata.upstream_version,
                PROGENITOR_SOURCE_VERSION_ASSUMPTION
            );
        }
    }

    fn expected_source_transform_diagnostic(
        metadata: SourceTransformMetadata,
        changed: bool,
        replacement_count: usize,
        status: SourceTransformStatus,
    ) -> BackendDiagnostic {
        BackendDiagnostic::SourceTransform(SourceTransformDiagnostic {
            name: metadata.name,
            changed,
            replacement_count,
            purpose: metadata.purpose,
            required: metadata.required,
            status,
            precondition: metadata.precondition,
            postcondition: metadata.postcondition,
            upstream_assumption: metadata.upstream_assumption,
            upstream_version: metadata.upstream_version,
        })
    }

    #[test]
    fn string_vec_value_parser_transform_handles_inline_shape() {
        let patched = patch_string_vec_value_parser(STRING_VEC_VALUE_PARSER_INLINE);

        assert!(!patched.contains(STRING_VEC_VALUE_PARSER_INLINE));
        assert!(patched.contains("s.split(',')"));
        assert!(patched.contains("Vec<::std::string::String>"));
    }

    #[test]
    fn string_vec_value_parser_transform_handles_pretty_shape() {
        let patched = patch_string_vec_value_parser(STRING_VEC_VALUE_PARSER_PRETTY);

        assert!(!patched.contains(STRING_VEC_VALUE_PARSER_PRETTY));
        assert!(patched.contains("s.split(',')"));
    }

    #[test]
    fn conditional_string_vec_transform_fails_when_residual_shape_remains() {
        let unsupported_shape = "::clap::value_parser!(::std::vec::Vec<::std::string::String >)";
        let error = apply_source_transform(
            STRING_VEC_VALUE_PARSER_METADATA,
            unsupported_shape,
            patch_string_vec_value_parser_with_count,
            string_vec_value_parser_postcondition_met,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("string_vec_value_parser' failed postcondition"));
    }

    #[test]
    fn unexpected_response_transform_preserves_body_text() {
        let patched = patch_unexpected_response_body(UNEXPECTED_RESPONSE_NEEDLE);

        assert!(!patched.contains(UNEXPECTED_RESPONSE_NEEDLE));
        assert!(patched.contains("response"));
        assert!(patched.contains(".text()"));
        assert!(patched.contains("Error::Custom"));
        assert!(patched.contains("body={body}"));
    }

    #[test]
    fn complex_single_value_parser_transform_accepts_json_or_string() {
        let patched = patch_complex_clap_value_parsers("::clap::value_parser!(types::Widget)");

        assert!(patched.contains("serde_json::from_value::<types::Widget>"));
        assert!(patched.contains("serde_json::from_str::<types::Widget>"));
        assert!(patched.contains("ValueParser::new"));
    }

    #[test]
    fn complex_vec_value_parser_transform_accepts_json_array_or_csv() {
        let patched = patch_complex_clap_value_parsers(
            "::clap::value_parser!(::std::vec::Vec < types::Widget >)",
        );

        assert!(patched.contains("Vec<types::Widget>"));
        assert!(patched.contains("from_str::<::std::vec::Vec<types::Widget>>"));
        assert!(patched.contains("s.split(',')"));
    }

    #[test]
    fn all_generated_source_transforms_compose_and_report_applied_counts() {
        let source = format!(
            "{STRING_VEC_VALUE_PARSER_INLINE}\n{UNEXPECTED_RESPONSE_NEEDLE}\n::clap::value_parser!(types::Widget)"
        );
        let transformed = apply_generated_source_transforms(&source).expect("transforms apply");
        let patched = &transformed.source;

        assert!(patched.contains("s.split(',')"));
        assert!(patched.contains("body={body}"));
        assert!(patched.contains("serde_json::from_value::<types::Widget>"));
        assert_eq!(
            transformed.diagnostics,
            vec![
                expected_source_transform_diagnostic(
                    STRING_VEC_VALUE_PARSER_METADATA,
                    true,
                    1,
                    SourceTransformStatus::Applied,
                ),
                expected_source_transform_diagnostic(
                    UNEXPECTED_RESPONSE_BODY_METADATA,
                    true,
                    1,
                    SourceTransformStatus::Applied,
                ),
                expected_source_transform_diagnostic(
                    COMPLEX_CLAP_VALUE_PARSERS_METADATA,
                    true,
                    1,
                    SourceTransformStatus::Applied,
                ),
            ]
        );
    }

    #[test]
    fn generated_source_transforms_report_conditional_checks_as_verified_not_needed() {
        let source = UNEXPECTED_RESPONSE_NEEDLE;
        let transformed =
            apply_generated_source_transforms(source).expect("required transform applies");

        assert!(transformed.source.contains("body={body}"));
        assert_eq!(
            transformed.diagnostics,
            vec![
                expected_source_transform_diagnostic(
                    STRING_VEC_VALUE_PARSER_METADATA,
                    false,
                    0,
                    SourceTransformStatus::VerifiedNotNeeded,
                ),
                expected_source_transform_diagnostic(
                    UNEXPECTED_RESPONSE_BODY_METADATA,
                    true,
                    1,
                    SourceTransformStatus::Applied,
                ),
                expected_source_transform_diagnostic(
                    COMPLEX_CLAP_VALUE_PARSERS_METADATA,
                    false,
                    0,
                    SourceTransformStatus::VerifiedNotNeeded,
                ),
            ]
        );
    }

    #[test]
    fn generated_source_transforms_report_required_transform_as_not_applicable_without_fallback() {
        let transformed = apply_generated_source_transforms("pub fn untouched() {}").unwrap();

        assert_eq!(
            transformed.diagnostics[1],
            expected_source_transform_diagnostic(
                UNEXPECTED_RESPONSE_BODY_METADATA,
                false,
                0,
                SourceTransformStatus::NotApplicable,
            )
        );
    }

    #[test]
    fn generated_source_transforms_fail_fast_when_required_shape_drifts() {
        let error =
            apply_generated_source_transforms("_ => Err(Error::UnexpectedResponse(response))")
                .unwrap_err();

        assert!(error
            .to_string()
            .contains("required generated source transform 'unexpected_response_body'"));
        assert!(error.to_string().contains("upstream drift"));
    }

    #[test]
    fn unexpected_response_transform_fails_when_partial_drift_remains() {
        let source =
            format!("{UNEXPECTED_RESPONSE_NEEDLE}\n_ => Err(Error::UnexpectedResponse(response))");
        let error = apply_generated_source_transforms(&source).unwrap_err();

        assert!(error
            .to_string()
            .contains("unexpected_response_body' failed postcondition"));
    }
}
