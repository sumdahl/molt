use clap::{CommandFactory, Parser};

/// Reclaim disk space on Linux, safely.
#[derive(Parser)]
#[command(version, about)]
struct Cli {}

fn main() -> std::io::Result<()> {
    Cli::parse();
    Cli::command().print_help()
}
