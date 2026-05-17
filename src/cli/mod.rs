use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use std::collections::{BTreeMap, BTreeSet};
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
    /// Check an OpenAPI spec against pp's strict native generation contract
    Check {
        /// Path to the OpenAPI 3.0/3.1 spec (YAML or JSON)
        spec: PathBuf,
        /// Emit machine-readable check JSON
        #[arg(long)]
        json: bool,
        /// Explicit base URL when the spec has no servers[0].url or when overriding it
        #[arg(long)]
        base_url: Option<String>,
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
    /// Query pp's support matrix and diagnostic-code inventory
    Support {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
        /// Query a single support feature by stable ID
        #[arg(long, conflicts_with = "diagnostic")]
        feature: Option<String>,
        /// Query a diagnostic code and the features that document it
        #[arg(long, conflicts_with = "feature")]
        diagnostic: Option<String>,
    },
    /// Explain a diagnostic code and how to address it
    Explain {
        /// Diagnostic code to explain, such as direct_http.request_body_json_missing
        diagnostic_code: String,
        /// Emit machine-readable explanation JSON
        #[arg(long)]
        json: bool,
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

fn print_support(json: bool, feature: Option<String>, diagnostic: Option<String>) -> Result<()> {
    match (feature, diagnostic) {
        (Some(feature_id), None) => {
            let feature = crate::support::feature_by_id(&feature_id)
                .ok_or_else(|| anyhow!("unknown support feature '{feature_id}'"))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "matrix_id": crate::support::SUPPORT_MATRIX_ID,
                        "feature": feature,
                    }))?
                );
            } else {
                println!("{}\t{:?}\t{}", feature.id, feature.status, feature.summary);
            }
        }
        (None, Some(code)) => {
            let payload = crate::support::features_for_diagnostic(&code)
                .ok_or_else(|| anyhow!("unknown diagnostic code '{code}'"))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("{}", payload.diagnostic_code);
                for feature in payload.features {
                    println!(
                        "  {}\t{:?}\t{}",
                        feature.id, feature.status, feature.summary
                    );
                }
            }
        }
        (None, None) => {
            let payload = crate::support::support_payload();
            if json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("{}", payload.matrix_id);
                for feature in payload.features {
                    println!("{}\t{:?}\t{}", feature.id, feature.status, feature.summary);
                }
            }
        }
        (Some(_), Some(_)) => unreachable!("clap rejects conflicting support filters"),
    }
    Ok(())
}

fn print_explain(diagnostic_code: String, json: bool) -> Result<()> {
    let explanation = crate::support::explain_diagnostic(&diagnostic_code)
        .ok_or_else(|| anyhow!("unknown diagnostic code '{diagnostic_code}'"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&explanation)?);
        return Ok(());
    }

    println!("pp explain: {}", explanation.diagnostic_code);
    println!("matrix: {}", explanation.matrix_id);
    println!();
    println!("{}", explanation.title);
    println!();
    println!("Meaning:");
    println!("  {}", explanation.meaning);
    println!();
    println!("Severity:");
    println!("  {}", explanation.severity_hint);
    println!();
    println!("Strict behavior:");
    println!("  {}", explanation.strict_behavior);
    println!();
    println!("Remediation:");
    println!("  {}", explanation.remediation);
    if !explanation.features.is_empty() {
        println!();
        println!("Related support features:");
        for feature in explanation.features {
            println!(
                "  {}\t{:?}\t{}",
                feature.id, feature.status, feature.summary
            );
        }
    }
    Ok(())
}

fn print_check_human(result: &crate::pipeline::CheckResult) {
    if result.success {
        println!("pp check: ok");
    } else {
        println!("pp check: failed");
    }

    if let Some(facts) = &result.facts {
        println!();
        println!("Spec:");
        println!("  title: {}", facts.title);
        println!("  operations: {}", facts.operation_count);
        println!("  binary: {}", facts.bin_name);
        println!(
            "  base url: {}",
            facts.base_url.as_deref().unwrap_or("none")
        );
        println!("  auth: {}", describe_auth_kind(&facts.auth_kind));
    }

    if !result.reports.is_empty() {
        println!();
        println!("Warnings:");
        for report in &result.reports {
            println!("  [{}] {}", report.code, report.message);
        }
    }

    if !result.diagnostics.is_empty() {
        println!();
        println!("Diagnostics:");
        for diagnostic in &result.diagnostics {
            println!(
                "  [{}] {} {}: {}",
                diagnostic.severity, diagnostic.code, diagnostic.source, diagnostic.message
            );
            if let Some(title) = &diagnostic.title {
                println!("    title: {title}");
            }
            if !diagnostic.support_features.is_empty() {
                println!(
                    "    related support features: {}",
                    diagnostic.support_features.join(", ")
                );
            }
            if let Some(strict_behavior) = &diagnostic.strict_behavior {
                println!("    strict behavior: {strict_behavior}");
            }
            if let Some(remediation) = &diagnostic.remediation {
                println!("    remediation: {remediation}");
            }
        }
    }

    if !result.unsupported_operations.is_empty() {
        println!();
        println!("Unsupported operations:");
        let mut operations_by_code: BTreeMap<
            &str,
            Vec<&crate::pipeline::CheckUnsupportedOperation>,
        > = BTreeMap::new();
        for operation in &result.unsupported_operations {
            operations_by_code
                .entry(operation.diagnostic_code.as_str())
                .or_default()
                .push(operation);
        }
        for (code, operations) in operations_by_code {
            let suffix = if operations.len() == 1 {
                "operation"
            } else {
                "operations"
            };
            println!("  {code} ({} {suffix})", operations.len());
            if let Some(first) = operations.first() {
                if !first.support_features.is_empty() {
                    println!(
                        "    related support features: {}",
                        first.support_features.join(", ")
                    );
                }
            }
            for operation in operations {
                let operation_id = operation.operation_id.as_deref().unwrap_or("none");
                println!(
                    "    {} {} (operationId: {})",
                    operation.method, operation.path, operation_id
                );
                println!("      reason: {}", operation.reason);
            }
        }
    }

    let diagnostic_codes = result
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<BTreeSet<_>>();
    if !diagnostic_codes.is_empty() {
        println!();
        println!("Explain diagnostics:");
        for code in diagnostic_codes {
            println!("  Run: pp explain {code}");
        }
    }
}

fn describe_auth_kind(auth_kind: &crate::spec::AuthKind) -> String {
    match auth_kind {
        crate::spec::AuthKind::None => "none".to_string(),
        crate::spec::AuthKind::Bearer => "http bearer".to_string(),
        crate::spec::AuthKind::HttpBasic => "http basic".to_string(),
        crate::spec::AuthKind::ApiKey { header_name } => format!("apiKey header ({header_name})"),
        crate::spec::AuthKind::QueryApiKey { param_name } => {
            format!("apiKey query parameter ({param_name})")
        }
        crate::spec::AuthKind::Unsupported { reason } => format!("unsupported ({reason})"),
    }
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Check {
                spec,
                json,
                base_url,
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
                let result = crate::pipeline::check(crate::pipeline::CheckRequest {
                    spec_path: spec,
                    base_url,
                    load_options: options,
                });
                if json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    print_check_human(&result);
                }
                if result.success {
                    Ok(())
                } else {
                    Err(anyhow!("check failed"))
                }
            }
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
            Command::Support {
                json,
                feature,
                diagnostic,
            } => print_support(json, feature, diagnostic),
            Command::Explain {
                diagnostic_code,
                json,
            } => print_explain(diagnostic_code, json),
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

    #[test]
    fn explain_accepts_diagnostic_code_and_json_flag() {
        let cli = Cli::parse_from([
            "pp",
            "explain",
            "direct_http.request_body_json_missing",
            "--json",
        ]);

        match cli.command {
            Command::Explain {
                diagnostic_code,
                json,
            } => {
                assert_eq!(diagnostic_code, "direct_http.request_body_json_missing");
                assert!(json);
            }
            _ => panic!("expected explain command"),
        }
    }
}
