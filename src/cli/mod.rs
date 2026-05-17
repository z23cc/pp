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

fn auth_policy_from_scheme(auth_scheme: Option<String>) -> crate::spec::AuthSelectionPolicy {
    if let Some(name) = auth_scheme {
        crate::spec::AuthSelectionPolicy::ExplicitScheme { name }
    } else {
        crate::spec::AuthSelectionPolicy::FailAmbiguous
    }
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
        /// Explicit component security scheme name to use for generated auth
        #[arg(long = "auth-scheme")]
        auth_scheme: Option<String>,
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
        /// Explicit component security scheme name to use for generated auth
        #[arg(long = "auth-scheme")]
        auth_scheme: Option<String>,
    },
    /// Run `cargo build` against an already-generated workspace
    Validate {
        /// Path to a generated workspace
        workspace: PathBuf,
    },
}

struct LoadOptionsArgs {
    include_operations: Vec<String>,
    include_tags: Vec<String>,
    include_path_prefixes: Vec<String>,
    exclude_operations: Vec<String>,
    auth_scheme: Option<String>,
}

fn load_options(args: LoadOptionsArgs) -> crate::spec::LoadOptions {
    let LoadOptionsArgs {
        include_operations,
        include_tags,
        include_path_prefixes,
        exclude_operations,
        auth_scheme,
    } = args;

    crate::spec::LoadOptions {
        slice: crate::spec::slice::SliceOptions {
            include_operations,
            include_tags,
            include_path_prefixes,
            exclude_operations,
        },
        auth_policy: auth_policy_from_scheme(auth_scheme),
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
        crate::pipeline::GenerateProgress::QueryApiKeyUsesExplicitParameter { param_name } => {
            eprintln!(
                "pp: query API key heuristic selected '{param_name}'; pass --{param_name} on generated CLI commands"
            );
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
                auth_scheme,
            } => {
                let options = load_options(LoadOptionsArgs {
                    include_operations,
                    include_tags,
                    include_path_prefixes,
                    exclude_operations,
                    auth_scheme,
                });
                if list_operations {
                    let loaded = crate::spec::load_for_operation_listing(&spec, &options)?;
                    for report in &loaded.reports {
                        eprintln!("pp: {}", report.formatted_warning());
                    }
                    for operation in crate::spec::slice::list_operations(&loaded.spec) {
                        println!("{}", serde_json::to_string(&operation)?);
                    }
                } else {
                    let loaded = crate::spec::load_with_options(&spec, &options)?;
                    if reports {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "facts": loaded.facts,
                                "auth_plan": loaded.auth_plan,
                                "reports": loaded.reports,
                            }))?
                        );
                    } else {
                        for report in &loaded.reports {
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
                auth_scheme,
            } => {
                let options = load_options(LoadOptionsArgs {
                    include_operations,
                    include_tags,
                    include_path_prefixes,
                    exclude_operations,
                    auth_scheme,
                });
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn load_options_auth_scheme_overrides_policy() {
        let options = load_options(LoadOptionsArgs {
            include_operations: Vec::new(),
            include_tags: Vec::new(),
            include_path_prefixes: Vec::new(),
            exclude_operations: Vec::new(),
            auth_scheme: Some("bearerAuth".to_string()),
        });

        assert!(matches!(
            options.auth_policy,
            crate::spec::AuthSelectionPolicy::ExplicitScheme { ref name }
                if name == "bearerAuth"
        ));
    }

    #[test]
    fn inspect_defaults_to_fail_ambiguous_auth_policy() {
        let cli = Cli::parse_from(["pp", "inspect", "spec.yaml"]);

        match cli.command {
            Command::Inspect { auth_scheme, .. } => {
                assert!(auth_scheme.is_none());
            }
            _ => panic!("expected inspect command"),
        }
    }

    #[test]
    fn generate_defaults_to_fail_ambiguous_auth_policy() {
        let cli = Cli::parse_from(["pp", "generate", "spec.yaml", "-o", "out"]);

        match cli.command {
            Command::Generate { auth_scheme, .. } => {
                assert!(auth_scheme.is_none());
            }
            _ => panic!("expected generate command"),
        }
    }

    #[test]
    fn inspect_rejects_removed_auth_policy_flag() {
        let err = Cli::try_parse_from([
            "pp",
            "inspect",
            "spec.yaml",
            "--auth-policy",
            "fail-ambiguous",
        ])
        .unwrap_err();

        assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn generate_accepts_explicit_auth_scheme_flag() {
        let cli = Cli::parse_from([
            "pp",
            "generate",
            "spec.yaml",
            "-o",
            "out",
            "--auth-scheme",
            "bearerAuth",
        ]);

        match cli.command {
            Command::Generate { auth_scheme, .. } => {
                assert_eq!(auth_scheme.as_deref(), Some("bearerAuth"));
            }
            _ => panic!("expected generate command"),
        }
    }
}
