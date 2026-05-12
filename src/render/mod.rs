//! Render the wrapper crate around progenitor's generated API crate.

use crate::spec::{AuthKind, OperationFacts};
use anyhow::{Context, Result};
use heck::ToShoutySnakeCase;
use minijinja::Environment;
use serde::Serialize;
use std::fs;
use std::path::Path;

const CARGO_TEMPLATE: &str = include_str!("templates/Cargo.toml.j2");
const API_CARGO_TEMPLATE: &str = include_str!("templates/api_cargo.toml.j2");
const MAIN_TEMPLATE: &str = include_str!("templates/main.rs.j2");
const CLI_BUILDER_TEMPLATE: &str = include_str!("templates/cli_builder.rs.j2");
const CONTEXT_TEMPLATE: &str = include_str!("templates/context.rs.j2");
const AUTH_TEMPLATE: &str = include_str!("templates/auth.rs.j2");
const PRINT_TEMPLATE: &str = include_str!("templates/print.rs.j2");

/// Facts required to render the generated wrapper crate.
#[derive(Debug, Clone, Serialize)]
pub struct WrapperManifest {
    pub bin_name: String,
    pub base_url: String,
    pub base_url_is_relative: bool,
    pub auth_kind: AuthKind,
    pub operations: Vec<OperationFacts>,
    pub progenitor_lib_name: String,
    pub progenitor_crate_name: String,
    pub token_env_var: String,
    pub api_key_env_var: String,
}

impl WrapperManifest {
    /// Build template data from inspected facts and the selected progenitor crate name.
    pub fn new(
        bin_name: String,
        base_url: Option<String>,
        base_url_is_relative: bool,
        auth_kind: AuthKind,
        operations: Vec<OperationFacts>,
        progenitor_lib_name: String,
    ) -> Self {
        let env_prefix = bin_name.to_shouty_snake_case();
        let progenitor_crate_name = progenitor_lib_name.replace('-', "_");
        Self {
            bin_name,
            base_url: base_url.unwrap_or_else(|| "http://localhost".to_string()),
            base_url_is_relative,
            auth_kind,
            operations,
            progenitor_lib_name,
            progenitor_crate_name,
            token_env_var: format!("{env_prefix}_TOKEN"),
            api_key_env_var: format!("{env_prefix}_API_KEY"),
        }
    }
}

/// Render all wrapper files into `out_dir`.
pub fn render(manifest: &WrapperManifest, out_dir: &Path) -> Result<()> {
    fs::create_dir_all(out_dir.join("src"))
        .with_context(|| format!("failed to create wrapper src dir: {}", out_dir.display()))?;

    write_template(
        "Cargo.toml",
        CARGO_TEMPLATE,
        manifest,
        &out_dir.join("Cargo.toml"),
    )?;
    write_template(
        "api/Cargo.toml",
        API_CARGO_TEMPLATE,
        manifest,
        &out_dir.join("api/Cargo.toml"),
    )?;
    write_template(
        "main.rs",
        MAIN_TEMPLATE,
        manifest,
        &out_dir.join("src/main.rs"),
    )?;
    write_template(
        "cli_builder.rs",
        CLI_BUILDER_TEMPLATE,
        manifest,
        &out_dir.join("src/cli_builder.rs"),
    )?;
    write_template(
        "context.rs",
        CONTEXT_TEMPLATE,
        manifest,
        &out_dir.join("src/context.rs"),
    )?;
    write_template(
        "auth.rs",
        AUTH_TEMPLATE,
        manifest,
        &out_dir.join("src/auth.rs"),
    )?;
    write_template(
        "print.rs",
        PRINT_TEMPLATE,
        manifest,
        &out_dir.join("src/print.rs"),
    )?;
    Ok(())
}

fn render_template(name: &str, source: &str, manifest: &WrapperManifest) -> Result<String> {
    let mut env = Environment::new();
    env.add_template(name, source)?;
    Ok(env.get_template(name)?.render(manifest)?)
}

fn write_template(name: &str, source: &str, manifest: &WrapperManifest, path: &Path) -> Result<()> {
    let rendered = render_template(name, source, manifest)?;
    fs::write(path, rendered).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_template_contains_workspace_and_api_dependency() {
        let manifest = WrapperManifest::new(
            "petstore".to_string(),
            Some("https://example.test".to_string()),
            false,
            AuthKind::None,
            vec![],
            "petstore-api".to_string(),
        );

        let rendered = render_template("Cargo.toml", CARGO_TEMPLATE, &manifest).unwrap();

        assert!(rendered.contains("[workspace]"));
        assert!(rendered.contains("members = [\"api\", \".\"]"));
        assert!(rendered.contains("name = \"petstore\""));
        assert!(rendered.contains("petstore-api = { path = \"api\" }"));
    }
}
