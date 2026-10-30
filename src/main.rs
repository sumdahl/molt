#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use molt::config::{Decision, DryRunCause, Requested};
use molt::startup;

/// Reclaim disk space on Linux, safely.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Show what would happen without changing anything.
    #[arg(long, global = true, conflicts_with = "live")]
    dry_run: bool,
    /// Act, even when the config or MOLT_DRY_RUN asks for a dry run.
    #[arg(long, global = true)]
    live: bool,
    /// Don't ask for confirmation.
    #[arg(long, global = true)]
    yes: bool,
    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,
}

impl Cli {
    fn requested(&self) -> Requested {
        match (self.dry_run, self.live) {
            (true, _) => Requested::DryRun,
            (_, true) => Requested::Live,
            _ => Requested::Default,
        }
    }
}

fn main() -> ExitCode {
    // P-13: clap exits 2 on bad usage, and 0 for --help and --version.
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // CL-03, CL-09
            eprintln!("molt: {error}");
            if cli.json {
                println!("{}", serde_json::json!({ "error": error.to_string() }));
            }
            ExitCode::from(1)
        }
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    let startup = startup::start(
        cli.requested(),
        |name| std::env::var_os(name),
        rustix::process::geteuid(),
    )?;
    // CL-06
    if let Decision::DryRun(cause @ (DryRunCause::Config | DryRunCause::Environment)) =
        startup.decision
    {
        let source = match cause {
            DryRunCause::Environment => "MOLT_DRY_RUN=1",
            _ => "dry_run = true in the config",
        };
        eprintln!("molt: dry run because of {source}; pass --live to act");
    }
    // No commands yet (005 adds scan), so show what molt accepts.
    Cli::command().print_help()?;
    Ok(())
}
