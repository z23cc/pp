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
                eprintln!("pp: inspecting {}...", spec.display());
                let loaded = crate::spec::load(&spec)?;
                for warning in &loaded.normalization_warnings {
                    eprintln!("pp: {warning}");
                }
                let facts = loaded.facts;
                let bin_name = name.unwrap_or(facts.bin_name);
                let api_name = format!("{bin_name}-api");
                eprintln!(
                    "pp: spec ok ({} operations, auth={:?}); target bin '{bin_name}'",
                    facts.operation_count, facts.auth_kind
                );
                let manifest = crate::render::WrapperManifest::new(
                    bin_name,
                    facts.base_url,
                    facts.base_url_is_relative,
                    facts.auth_kind,
                    api_name.clone(),
                );
                if let crate::spec::AuthKind::QueryApiKey { param_name } = &manifest.auth_kind {
                    eprintln!(
                        "pp: query API key '{param_name}' auto-injection is limited — users may still need --{param_name} on the command line"
                    );
                }

                eprintln!("pp: generating API crate via progenitor...");
                crate::progenitor_driver::generate(&loaded.api, &output.join("api"), &api_name)?;
                eprintln!("pp: rendering wrapper crate...");
                crate::render::render(&manifest, &output)?;
                eprintln!("pp: workspace written to {}", output.display());

                if build {
                    eprintln!("pp: running `cargo build --release` (this can take 1-2 minutes)...");
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
                    eprintln!("pp: build succeeded");
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
