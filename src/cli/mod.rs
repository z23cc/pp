use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

#[derive(Parser, Debug)]
#[command(
    name = "pp",
    about = "Printing Press: OpenAPI -> installable Rust CLI",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Inspect an OpenAPI spec and print the derived facts as JSON
    Inspect {
        /// Path to the OpenAPI 3.0 spec (YAML or JSON)
        spec: PathBuf,
    },
    /// Generate a Rust CLI crate workspace from an OpenAPI spec
    Generate {
        /// Path to the OpenAPI 3.0 spec (YAML or JSON)
        spec: PathBuf,
        /// Output directory (will be created)
        #[arg(short, long)]
        output: PathBuf,
        /// Override the binary name (default: derived from info.title)
        #[arg(short, long)]
        name: Option<String>,
        /// Run `cargo build --release` after generation to validate
        #[arg(long)]
        build: bool,
    },
    /// Run `cargo build` against an already-generated workspace
    Validate {
        /// Path to a generated workspace
        workspace: PathBuf,
    },
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Inspect { spec } => {
                let facts = crate::spec::inspect(&spec)?;
                println!("{}", serde_json::to_string_pretty(&facts)?);
                Ok(())
            }
            Command::Generate {
                spec,
                output,
                name,
                build,
            } => {
                let facts = crate::spec::inspect(&spec)?;
                let bin_name = name.unwrap_or(facts.bin_name);
                let api_name = format!("{bin_name}-api");
                let manifest = crate::render::WrapperManifest::new(
                    bin_name,
                    facts.base_url,
                    facts.base_url_is_relative,
                    facts.auth_kind,
                    api_name.clone(),
                );

                let progenitor_version = crate::progenitor_driver::check_available()?;
                if !progenitor_version.contains(crate::progenitor_driver::PINNED_VERSION) {
                    eprintln!(
                        "warning: expected cargo-progenitor {}, found {progenitor_version}",
                        crate::progenitor_driver::PINNED_VERSION
                    );
                }
                crate::progenitor_driver::generate(&spec, &output.join("api"), &api_name)?;
                crate::render::render(&manifest, &output)?;

                if build {
                    let out = ProcessCommand::new("cargo")
                        .arg("build")
                        .arg("--release")
                        .current_dir(&output)
                        .output()
                        .with_context(|| {
                            format!("failed to spawn cargo build in {}", output.display())
                        })?;
                    if !out.status.success() {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        return Err(anyhow!(
                            "cargo build --release failed (exit {}):\n{stderr}",
                            out.status.code().unwrap_or(-1)
                        ));
                    }
                }

                Ok(())
            }
            Command::Validate { workspace } => {
                let _ = workspace;
                anyhow::bail!("`validate` not implemented yet (Week 3 work item 9)");
            }
        }
    }
}
