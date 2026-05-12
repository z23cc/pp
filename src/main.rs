mod cli;
mod spec;
mod progenitor_driver;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    cli.run()
}
