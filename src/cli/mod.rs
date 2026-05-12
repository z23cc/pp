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
            Command::Generate { spec, output, name, build } => {
                let _ = (spec, output, name, build);
                anyhow::bail!("`generate` not implemented yet (Week 2 work items 4-8)");
            }
            Command::Validate { workspace } => {
                let _ = workspace;
                anyhow::bail!("`validate` not implemented yet (Week 3 work item 9)");
            }
        }
    }
}
