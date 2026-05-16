//! In-process driver around the `progenitor` library.

use crate::backend::{ApiCrateOutput, BackendDiagnostic};
use anyhow::{Context, Result};
use openapiv3::OpenAPI;
use progenitor::{GenerationSettings, Generator, InterfaceStyle};
use quote::{format_ident, quote};
use regex::Regex;
use std::fs;
use std::path::Path;

const TRANSFORM_STRING_VEC_VALUE_PARSER: &str = "string_vec_value_parser";
const TRANSFORM_UNEXPECTED_RESPONSE_BODY: &str = "unexpected_response_body";
const TRANSFORM_COMPLEX_CLAP_VALUE_PARSERS: &str = "complex_clap_value_parsers";

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
    let transformed = apply_generated_source_transforms(&formatted);

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

fn apply_generated_source_transforms(source: &str) -> GeneratedSourceTransformOutput {
    let mut source = source.to_string();
    let mut diagnostics = Vec::new();

    let (next, replacement_count) = patch_string_vec_value_parser_with_count(&source);
    diagnostics.push(source_transform_diagnostic(
        TRANSFORM_STRING_VEC_VALUE_PARSER,
        &source,
        &next,
        replacement_count,
    ));
    source = next;

    let (next, replacement_count) = patch_unexpected_response_body_with_count(&source);
    diagnostics.push(source_transform_diagnostic(
        TRANSFORM_UNEXPECTED_RESPONSE_BODY,
        &source,
        &next,
        replacement_count,
    ));
    source = next;

    let (next, replacement_count) = patch_complex_clap_value_parsers_with_count(&source);
    diagnostics.push(source_transform_diagnostic(
        TRANSFORM_COMPLEX_CLAP_VALUE_PARSERS,
        &source,
        &next,
        replacement_count,
    ));

    GeneratedSourceTransformOutput {
        source: next,
        diagnostics,
    }
}

fn source_transform_diagnostic(
    name: &'static str,
    before: &str,
    after: &str,
    replacement_count: usize,
) -> BackendDiagnostic {
    BackendDiagnostic::SourceTransform {
        name,
        changed: before != after,
        replacement_count,
    }
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

#[cfg(test)]
fn patch_unexpected_response_body(source: &str) -> String {
    patch_unexpected_response_body_with_count(source).0
}

fn patch_unexpected_response_body_with_count(source: &str) -> (String, usize) {
    let replacement_count = source.matches(UNEXPECTED_RESPONSE_NEEDLE).count();
    let patched = source.replace(UNEXPECTED_RESPONSE_NEEDLE, UNEXPECTED_RESPONSE_REPLACEMENT);
    (patched, replacement_count)
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
        let transformed = apply_generated_source_transforms(&source);
        let patched = &transformed.source;

        assert!(patched.contains("s.split(',')"));
        assert!(patched.contains("body={body}"));
        assert!(patched.contains("serde_json::from_value::<types::Widget>"));
        assert_eq!(
            transformed.diagnostics,
            vec![
                BackendDiagnostic::SourceTransform {
                    name: TRANSFORM_STRING_VEC_VALUE_PARSER,
                    changed: true,
                    replacement_count: 1,
                },
                BackendDiagnostic::SourceTransform {
                    name: TRANSFORM_UNEXPECTED_RESPONSE_BODY,
                    changed: true,
                    replacement_count: 1,
                },
                BackendDiagnostic::SourceTransform {
                    name: TRANSFORM_COMPLEX_CLAP_VALUE_PARSERS,
                    changed: true,
                    replacement_count: 1,
                },
            ]
        );
    }

    #[test]
    fn generated_source_transforms_report_skipped_counts() {
        let source = "pub fn untouched() {}";
        let transformed = apply_generated_source_transforms(source);

        assert_eq!(transformed.source, source);
        assert_eq!(
            transformed.diagnostics,
            vec![
                BackendDiagnostic::SourceTransform {
                    name: TRANSFORM_STRING_VEC_VALUE_PARSER,
                    changed: false,
                    replacement_count: 0,
                },
                BackendDiagnostic::SourceTransform {
                    name: TRANSFORM_UNEXPECTED_RESPONSE_BODY,
                    changed: false,
                    replacement_count: 0,
                },
                BackendDiagnostic::SourceTransform {
                    name: TRANSFORM_COMPLEX_CLAP_VALUE_PARSERS,
                    changed: false,
                    replacement_count: 0,
                },
            ]
        );
    }
}
