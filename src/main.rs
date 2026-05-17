mod backend;
mod cli;
mod model;
mod pipeline;
mod render;
mod spec;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    cli.run()
}
