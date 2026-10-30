#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use molt::config::Requested;
use molt::scan;
use molt::startup::{self, Startup};

/// Reclaim disk space on Linux, safely.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
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

#[derive(Clone, Copy, Subcommand)]
enum Command {
    /// Show how much space molt could free, without changing anything (the default).
    Scan,
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
        Ok(code) => code,
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

fn run(cli: &Cli) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let startup = startup::start(
        cli.requested(),
        |name| std::env::var_os(name),
        rustix::process::geteuid(),
    )?;
    // SC-09
    match cli.command.unwrap_or(Command::Scan) {
        Command::Scan => run_scan(cli, &startup),
    }
}

fn run_scan(cli: &Cli, startup: &Startup) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let report = scan::scan(startup)?;
    if cli.json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        print!("{report}");
    }
    // SC-07, P-13
    Ok(if report.partial {
        ExitCode::from(3)
    } else {
        ExitCode::SUCCESS
    })
}
