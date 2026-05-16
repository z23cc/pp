use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
        /// Print JSON with facts plus structured preparation report entries
        #[arg(long, conflicts_with = "list_operations")]
        reports: bool,
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
        /// Permit compatibility rewrites/drops instead of failing strict normalization policy
        #[arg(long)]
        allow_compat_normalization: bool,
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
        /// Explicit base URL when the spec has no servers[0].url or when overriding it
        #[arg(long)]
        base_url: Option<String>,
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
        /// Permit compatibility rewrites/drops instead of failing strict normalization policy
        #[arg(long)]
        allow_compat_normalization: bool,
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
    allow_compat_normalization: bool,
) -> crate::spec::LoadOptions {
    crate::spec::LoadOptions {
        slice: crate::spec::slice::SliceOptions {
            include_operations,
            include_tags,
            include_path_prefixes,
            exclude_operations,
        },
        policy: if allow_compat_normalization {
            crate::spec::NormalizationPolicy::compatibility()
        } else {
            crate::spec::NormalizationPolicy::strict()
        },
    }
}

fn print_generate_progress(event: crate::pipeline::GenerateProgress) {
    match event {
        crate::pipeline::GenerateProgress::Inspecting { spec_path } => {
            eprintln!("pp: inspecting {}...", spec_path.display());
        }
        crate::pipeline::GenerateProgress::Warning { warning } => {
            eprintln!("pp: {warning}");
        }
        crate::pipeline::GenerateProgress::SpecOk {
            operation_count,
            auth_kind,
            target_bin_name,
        } => {
            eprintln!(
                "pp: spec ok ({} operations, auth={:?}); target bin '{target_bin_name}'",
                operation_count, auth_kind
            );
        }
        crate::pipeline::GenerateProgress::QueryApiKeyAutoInjectionLimited { param_name } => {
            eprintln!(
                "pp: query API key '{param_name}' auto-injection is limited — users may still need --{param_name} on the command line"
            );
        }
        crate::pipeline::GenerateProgress::GeneratingApiCrate => {
            eprintln!("pp: generating API crate via progenitor...");
        }
        crate::pipeline::GenerateProgress::RenderingWrapperCrate => {
            eprintln!("pp: rendering wrapper crate...");
        }
        crate::pipeline::GenerateProgress::WorkspaceWritten { output_path } => {
            eprintln!("pp: workspace written to {}", output_path.display());
        }
        crate::pipeline::GenerateProgress::BuildStarted => {
            eprintln!("pp: running `cargo build --release` (this can take 1-2 minutes)...");
        }
        crate::pipeline::GenerateProgress::BuildSucceeded => {
            eprintln!("pp: build succeeded");
        }
    }
}

fn validate_workspace_build(workspace: &std::path::Path) -> Result<()> {
    eprintln!("pp: running `cargo build --release` (this can take 1-2 minutes)...");
    crate::pipeline::validate_workspace_build(workspace)?;
    eprintln!("pp: build succeeded");
    Ok(())
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Inspect {
                spec,
                list_operations,
                reports,
                include_operations,
                include_tags,
                include_path_prefixes,
                exclude_operations,
                allow_compat_normalization,
            } => {
                let options = load_options(
                    include_operations,
                    include_tags,
                    include_path_prefixes,
                    exclude_operations,
                    allow_compat_normalization,
                );
                if list_operations {
                    let mut options = options.clone();
                    options.policy = crate::spec::NormalizationPolicy::compatibility();
                    let loaded = crate::spec::load_with_options(&spec, &options)?;
                    for report in &loaded.reports {
                        eprintln!("pp: {}", report.formatted_warning());
                    }
                    for operation in crate::spec::slice::list_operations(&loaded.api) {
                        println!("{}", serde_json::to_string(&operation)?);
                    }
                } else {
                    let loaded = crate::spec::load_with_options(&spec, &options)?;
                    if reports {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "facts": loaded.facts,
                                "reports": loaded.reports,
                            }))?
                        );
                    } else {
                        for report in loaded.reports.iter().filter(|report| {
                            report.stage == crate::spec::report::ReportStage::PreParseTolerance
                                || report.code
                                    == crate::spec::normalization_rules::typed::OPERATION_IDS_SHORTENED
                        }) {
                            eprintln!("pp: {}", report.formatted_warning());
                        }
                        println!("{}", serde_json::to_string_pretty(&loaded.facts)?);
                    }
                }
                Ok(())
            }
            Command::Generate {
                spec,
                output,
                name,
                base_url,
                build,
                include_operations,
                include_tags,
                include_path_prefixes,
                exclude_operations,
                allow_compat_normalization,
            } => {
                let options = load_options(
                    include_operations,
                    include_tags,
                    include_path_prefixes,
                    exclude_operations,
                    allow_compat_normalization,
                );
                let _result = crate::pipeline::generate_with_progress(
                    crate::pipeline::GenerateRequest {
                        spec_path: spec,
                        output_path: output,
                        bin_name: name,
                        base_url,
                        validate: build,
                        load_options: options,
                    },
                    print_generate_progress,
                )?;
                Ok(())
            }
            Command::Validate { workspace } => validate_workspace_build(&workspace),
        }
    }
}
