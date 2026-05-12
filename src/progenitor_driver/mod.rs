//! In-process driver around the `progenitor` library.

use anyhow::{Context, Result};
use openapiv3::OpenAPI;
use progenitor::{GenerationSettings, Generator, InterfaceStyle};
use quote::{format_ident, quote};
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
        );

    fs::create_dir_all(out_dir.join("src"))
        .with_context(|| format!("failed to create API src dir: {}", out_dir.display()))?;
    fs::write(out_dir.join("src/lib.rs"), formatted)
        .with_context(|| format!("failed to write {}", out_dir.join("src/lib.rs").display()))
}
