mod backend;
mod cli;
mod model;
mod pipeline;
mod progenitor_driver;
mod render;
mod spec;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    cli.run()
}
