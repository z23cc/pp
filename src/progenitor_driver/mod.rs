//! In-process driver around the `progenitor` library.

use anyhow::{Context, Result};
use openapiv3::OpenAPI;
use progenitor::{GenerationSettings, Generator, InterfaceStyle};
use quote::{format_ident, quote};
use std::fs;
use std::path::Path;

/// Generate a complete API crate containing progenitor's client and CLI tokens.
pub fn generate(spec: &Path, out_dir: &Path, crate_name: &str) -> Result<()> {
    let raw = fs::read_to_string(spec)
        .with_context(|| format!("failed to read spec: {}", spec.display()))?;
    let api: OpenAPI = parse_openapi(&raw)
        .with_context(|| format!("failed to parse spec: {}", spec.display()))?;

    let mut settings = GenerationSettings::default();
    settings
        .with_interface(InterfaceStyle::Builder)
        .with_derive("schemars::JsonSchema")
        .with_cli_bounds("schemars::JsonSchema")
        .with_cli_bounds("serde::Serialize")
        .with_cli_bounds("std::fmt::Debug");
    let mut generator = Generator::new(&settings);
    let client_tokens = generator.generate_tokens(&api)?;
    let crate_ident = format_ident!("{}", crate_name.replace('-', "_"));
    let cli_tokens = generator.cli(&api, &crate_ident.to_string())?;

    let file_tokens = quote! {
        extern crate self as #crate_ident;

        #client_tokens

        pub mod cli {
            #cli_tokens
        }
    };
    let syntax = syn::parse2(file_tokens)?;
    let formatted = prettyplease::unparse(&syntax);

    fs::create_dir_all(out_dir.join("src"))
        .with_context(|| format!("failed to create API src dir: {}", out_dir.display()))?;
    fs::write(out_dir.join("src/lib.rs"), formatted)
        .with_context(|| format!("failed to write {}", out_dir.join("src/lib.rs").display()))
}

fn parse_openapi(raw: &str) -> Result<OpenAPI> {
    let trimmed = raw.trim_start();
    if trimmed.starts_with('{') {
        Ok(serde_json::from_str(raw)?)
    } else {
        Ok(serde_yaml::from_str(raw)?)
    }
}
