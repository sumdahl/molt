#![forbid(unsafe_code)]

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use molt::clean::{self, Busy};
use molt::config::{Decision, Requested};
use molt::processes;
use molt::safe_delete::{Mode, Permissions};
use molt::scan::{self, Survey};
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

#[derive(Clone, Subcommand)]
enum Command {
    /// Show how much space molt could free, without changing anything (the default).
    Scan,
    /// Free the space `scan` shows: preview first, then ask, then clean.
    Clean(CleanArgs),
}

#[derive(Clone, Args)]
struct CleanArgs {
    /// Clean only these rules (ids as in `molt scan --json`).
    #[arg(long, num_args = 1.., value_name = "ID")]
    only: Vec<String>,
    /// Leave these rules out.
    #[arg(long, num_args = 1.., value_name = "ID")]
    skip: Vec<String>,
    /// Delete permanently, whatever each rule's default.
    #[arg(long, conflicts_with = "trash")]
    permanent: bool,
    /// Move to the trash, whatever each rule's default.
    #[arg(long)]
    trash: bool,
    /// Allow opening locked folders you own (u+rwx) so they can be removed.
    #[arg(long)]
    fix_permissions: bool,
}

impl CleanArgs {
    fn mode(&self) -> Option<Mode> {
        match (self.permanent, self.trash) {
            (true, _) => Some(Mode::Permanent),
            (_, true) => Some(Mode::Trash),
            _ => None,
        }
    }
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
    match &cli.command {
        None | Some(Command::Scan) => run_scan(cli, &startup),
        Some(Command::Clean(args)) => run_clean(cli, args, &startup),
    }
}

fn run_clean(
    cli: &Cli,
    args: &CleanArgs,
    startup: &Startup,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let rules = match clean::select(&startup.rules, &args.only, &args.skip) {
        Ok(rules) => rules,
        Err(unknown) => {
            // CN-02: a typo must not clean more than meant.
            eprintln!("molt: no enabled rule has the id `{unknown}`; nothing changed");
            return Ok(ExitCode::from(2));
        }
    };
    let (rules, busy) = clean::leave_out_busy(rules, &processes::running());
    let preview = scan::survey(&rules, startup, args.mode())?;
    print_preview(cli, &preview, &busy)?;

    // CN-07
    if let Decision::DryRun(_) = startup.decision {
        if let Some(notice) = startup::dry_run_notice(startup.decision) {
            eprintln!("molt: {notice}");
        }
        return Ok(ExitCode::SUCCESS);
    }
    if preview.report.rules.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    // CN-05
    let interactive = !cli.yes;
    if interactive {
        if !std::io::stdin().is_terminal() {
            return Err(
                "no terminal to ask for confirmation; pass --yes to clean. Nothing changed".into(),
            );
        }
        if !confirm("Clean all of the above?")? {
            println!("Nothing changed.");
            return Ok(ExitCode::SUCCESS);
        }
    }
    // CN-06, LF-05: a separate consent, never implied by --yes.
    let locked: usize = preview
        .report
        .rules
        .iter()
        .map(|rule| rule.locked.len())
        .sum();
    let permissions = if locked > 0
        && (args.fix_permissions
            || (interactive
                && confirm(&format!(
                    "Open the {locked} locked folders (add u+rwx) so they can be removed too?"
                ))?)) {
        Permissions::FixOwned
    } else {
        Permissions::Keep
    };

    let result = clean::act(&preview, &rules, startup, permissions)?;
    if cli.json {
        println!("{}", serde_json::json!({ "result": result }));
    } else {
        print!("{result}");
    }
    Ok(ExitCode::from(result.exit_code()))
}

/// P-14, CN-04: printed in full before anything changes, `--json` included.
fn print_preview(
    cli: &Cli,
    preview: &Survey,
    busy: &[Busy],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut out = std::io::stdout().lock();
    if cli.json {
        let preview = serde_json::json!({ "preview": { "report": preview.report, "busy": busy } });
        writeln!(out, "{preview}")?;
    } else {
        writeln!(out, "Preview. Nothing has changed yet:\n")?;
        write!(out, "{}", preview.report)?;
        for left_out in busy {
            writeln!(
                out,
                "Left out while {} is running: {}",
                left_out.program, left_out.name
            )?;
        }
    }
    out.flush()?;
    Ok(())
}

/// Asks on stderr; only `y` or `yes` is a yes (CN-05).
fn confirm(question: &str) -> std::io::Result<bool> {
    eprint!("{question} [y/N] ");
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
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
