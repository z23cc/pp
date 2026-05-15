//! In-process driver around the `progenitor` library.

use anyhow::{Context, Result};
use openapiv3::OpenAPI;
use progenitor::{GenerationSettings, Generator, InterfaceStyle};
use quote::{format_ident, quote};
use regex::Regex;
use std::fs;
use std::path::Path;

/// Generate a complete API crate containing progenitor's client and CLI tokens.
pub fn generate(api: &OpenAPI, out_dir: &Path, crate_name: &str) -> Result<()> {
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
    let formatted = prettyplease::unparse(&syntax)
        .replace(
            "::clap::value_parser!(::std::vec::Vec<::std::string::String>)",
            "::clap::builder::ValueParser::new(|s: &str| -> Result<::std::vec::Vec<::std::string::String>, ::std::string::String> { Ok(s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()) })",
        )
        .replace(
            "::clap::value_parser!(\n                                ::std::vec::Vec < ::std::string::String >\n                            )",
            "::clap::builder::ValueParser::new(|s: &str| -> Result<::std::vec::Vec<::std::string::String>, ::std::string::String> { Ok(s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()) })",
        )
        .replace(
            "_ => Err(Error::UnexpectedResponse(response)),",
            r#"_ => {
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
            }"#,
        );
    let formatted = patch_complex_clap_value_parsers(&formatted);

    fs::create_dir_all(out_dir.join("src"))
        .with_context(|| format!("failed to create API src dir: {}", out_dir.display()))?;
    fs::write(out_dir.join("src/lib.rs"), formatted)
        .with_context(|| format!("failed to write {}", out_dir.join("src/lib.rs").display()))
}

fn patch_complex_clap_value_parsers(source: &str) -> String {
    let vec_re = Regex::new(
        r"::clap::value_parser!\(\s*::std::vec::Vec\s*<\s*types::([A-Za-z][A-Za-z0-9_]*)\s*>\s*\)",
    )
    .expect("valid vec value parser regex");
    let source = vec_re.replace_all(source, |caps: &regex::Captures<'_>| {
        complex_vec_value_parser(&caps[1])
    });

    let single_re = Regex::new(r"::clap::value_parser!\(\s*types::([A-Za-z][A-Za-z0-9_]*)\s*\)")
        .expect("valid single value parser regex");
    single_re
        .replace_all(&source, |caps: &regex::Captures<'_>| {
            complex_single_value_parser(&caps[1])
        })
        .into_owned()
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
