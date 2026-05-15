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
        /// Print stable JSONL rows for operations after any slice filters
        #[arg(long)]
        list_operations: bool,
        /// Include an operation by operationId (repeatable)
        #[arg(long = "include-operation")]
        include_operations: Vec<String>,
        /// Include operations with this tag (repeatable)
        #[arg(long = "include-tag")]
        include_tags: Vec<String>,
        /// Include operations whose path starts with this prefix (repeatable)
        #[arg(long = "include-path-prefix")]
        include_path_prefixes: Vec<String>,
        /// Exclude an operation by operationId after includes are applied (repeatable)
        #[arg(long = "exclude-operation")]
        exclude_operations: Vec<String>,
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
        /// Include an operation by operationId (repeatable)
        #[arg(long = "include-operation")]
        include_operations: Vec<String>,
        /// Include operations with this tag (repeatable)
        #[arg(long = "include-tag")]
        include_tags: Vec<String>,
        /// Include operations whose path starts with this prefix (repeatable)
        #[arg(long = "include-path-prefix")]
        include_path_prefixes: Vec<String>,
        /// Exclude an operation by operationId after includes are applied (repeatable)
        #[arg(long = "exclude-operation")]
        exclude_operations: Vec<String>,
    },
    /// Run `cargo build` against an already-generated workspace
    Validate {
        /// Path to a generated workspace
        workspace: PathBuf,
    },
}

fn load_options(
    include_operations: Vec<String>,
    include_tags: Vec<String>,
    include_path_prefixes: Vec<String>,
    exclude_operations: Vec<String>,
) -> crate::spec::LoadOptions {
    crate::spec::LoadOptions {
        slice: crate::spec::slice::SliceOptions {
            include_operations,
            include_tags,
            include_path_prefixes,
            exclude_operations,
        },
    }
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Inspect {
                spec,
                list_operations,
                include_operations,
                include_tags,
                include_path_prefixes,
                exclude_operations,
            } => {
                let options = load_options(
                    include_operations,
                    include_tags,
                    include_path_prefixes,
                    exclude_operations,
                );
                if list_operations {
                    let loaded = crate::spec::load_with_options(&spec, &options)?;
                    for warning in &loaded.normalization_warnings {
                        eprintln!("pp: {warning}");
                    }
                    for operation in crate::spec::slice::list_operations(&loaded.api) {
                        println!("{}", serde_json::to_string(&operation)?);
                    }
                } else {
                    let facts = if options.slice.is_noop() {
                        crate::spec::inspect(&spec)?
                    } else {
                        crate::spec::inspect_with_options(&spec, &options)?
                    };
                    println!("{}", serde_json::to_string_pretty(&facts)?);
                }
                Ok(())
            }
            Command::Generate {
                spec,
                output,
                name,
                build,
                include_operations,
                include_tags,
                include_path_prefixes,
                exclude_operations,
            } => {
                eprintln!("pp: inspecting {}...", spec.display());
                let options = load_options(
                    include_operations,
                    include_tags,
                    include_path_prefixes,
                    exclude_operations,
                );
                let loaded = if options.slice.is_noop() {
                    crate::spec::load(&spec)?
                } else {
                    crate::spec::load_with_options(&spec, &options)?
                };
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
                )
                .with_openapi(&loaded.api)?;
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
