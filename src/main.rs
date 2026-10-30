#![forbid(unsafe_code)]

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;
use std::time::SystemTime;

use clap::{Args, Parser, Subcommand};
use molt::clean::{self, Busy};
use molt::config::{Decision, Requested};
use molt::processes;
use molt::purge;
use molt::rules::{Rule, RuleId};
use molt::safe_delete::{Mode, Permissions};
use molt::scan::{self, Survey};
use molt::startup::{self, Startup};
use molt::system;

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
    /// Remove build folders (target, node_modules, .venv…) of projects under purge_paths:
    /// preview first, then ask.
    Purge(PurgeArgs),
    /// Free system space (pacman cache, journal, coredumps) through each tool's own
    /// command, run with sudo: preview first, then ask.
    System(SystemArgs),
}

#[derive(Clone, Args)]
struct PurgeArgs {
    /// Only these kinds: cargo, node, python-venv, gradle, maven, next, zig.
    #[arg(long, num_args = 1.., value_name = "KIND")]
    only: Vec<String>,
    /// Leave these kinds out.
    #[arg(long, num_args = 1.., value_name = "KIND")]
    skip: Vec<String>,
    /// Move to the trash instead of deleting.
    #[arg(long)]
    trash: bool,
}

#[derive(Clone, Args)]
struct SystemArgs {
    /// Run only these tasks: pacman-cache, journal, coredumps.
    #[arg(long, num_args = 1.., value_name = "ID")]
    only: Vec<String>,
    /// Leave these tasks out.
    #[arg(long, num_args = 1.., value_name = "ID")]
    skip: Vec<String>,
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
        Some(Command::Purge(args)) => run_purge(cli, args, &startup),
        Some(Command::System(args)) => run_system(cli, args, &startup),
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
    ask_and_act(cli, startup, &preview, &rules, args.fix_permissions)
}

/// PG-01..PG-08: finds build folders under `purge_paths`, then goes on as `clean` does.
fn run_purge(
    cli: &Cli,
    args: &PurgeArgs,
    startup: &Startup,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let all = purge::rules();
    let kinds = match clean::select(&all, &args.only, &args.skip) {
        Ok(kinds) => kinds,
        Err(unknown) => {
            // PG-08
            eprintln!("molt: there is no project kind `{unknown}`; nothing changed");
            return Ok(ExitCode::from(2));
        }
    };
    let home = &startup.env.dirs.home;
    let roots: Vec<_> = startup
        .config
        .purge_paths
        .iter()
        .map(|path| path.resolve(home))
        .collect();
    // PG-01
    if roots.is_empty() {
        eprintln!(
            "molt: no purge_paths set. Add the folders holding your projects to {}, \
             e.g. purge_paths = [\"~/projects\"]",
            startup.env.dirs.config.join("molt/config.toml").display()
        );
    }
    let mut findings = purge::find(&roots, &startup.env.search_path, SystemTime::now());
    let picked = |rule: &RuleId| kinds.iter().any(|kind| kind.id == *rule);
    findings.matches.retain(|found| picked(&found.rule));
    // Walk errors belong to no kind, so they stay.
    let is_kind = |rule: &RuleId| all.iter().any(|kind| kind.id == *rule);
    findings
        .skipped
        .retain(|skip| picked(&skip.rule) || !is_kind(&skip.rule));
    let mut preview =
        scan::measure_findings(&kinds, findings, startup, args.trash.then_some(Mode::Trash))?;
    preview.report.freed_by = "molt purge";
    print_preview(cli, &preview, &[])?;
    if !cli.json {
        print_left_out(&preview);
    }
    ask_and_act(cli, startup, &preview, &kinds, false)
}

/// PG-04, PG-05: how many projects the preview left out, and why.
fn print_left_out(preview: &Survey) {
    for (reason, why) in [
        ("changed-in-the-last-7-days", "changed in the last 7 days"),
        ("git-tracked", "may hold Git-tracked files"),
    ] {
        let count = preview
            .report
            .skipped
            .iter()
            .filter(|skip| skip.reason == reason)
            .count();
        if count > 0 {
            println!("Left out {count} build folders: {why}. See --json.");
        }
    }
}

/// CN-05 to CN-07, CN-11: after the preview, the shared consent and the live run.
fn ask_and_act(
    cli: &Cli,
    startup: &Startup,
    preview: &Survey,
    rules: &[Rule],
    fix_permissions: bool,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    // CN-07
    if stop_for_dry_run(startup) || preview.report.rules.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    // CN-05
    if !go_ahead(cli, "Clean all of the above?")? {
        return Ok(ExitCode::SUCCESS);
    }
    let interactive = !cli.yes;
    // CN-06, LF-05: a separate consent, never implied by --yes.
    let locked: usize = preview
        .report
        .rules
        .iter()
        .map(|rule| rule.locked.len())
        .sum();
    let permissions = if locked > 0
        && (fix_permissions
            || (interactive
                && confirm(&format!(
                    "Open the {locked} locked folders (add u+rwx) so they can be removed too?"
                ))?)) {
        Permissions::FixOwned
    } else {
        Permissions::Keep
    };

    let result = clean::act(preview, rules, startup, permissions)?;
    if cli.json {
        println!("{}", serde_json::json!({ "result": result }));
    } else {
        print!("{result}");
    }
    Ok(ExitCode::from(result.exit_code()))
}

/// SY-01..SY-11: previews every task, asks once, then runs each through sudo.
fn run_system(
    cli: &Cli,
    args: &SystemArgs,
    startup: &Startup,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let search_path = &startup.env.search_path;
    let tasks = system::detect(search_path);
    let selected = match system::select(tasks.active.clone(), &args.only, &args.skip) {
        Ok(selected) => selected,
        Err(unknown) => {
            eprintln!("molt: there is no system task `{unknown}`; nothing changed");
            return Ok(ExitCode::from(2));
        }
    };
    let root = std::path::Path::new("/");
    let preview = system::preview(&tasks, &selected, root, search_path);
    let mut out = std::io::stdout().lock();
    if cli.json {
        writeln!(out, "{}", serde_json::json!({ "preview": preview }))?;
    } else {
        writeln!(out, "Preview. Nothing has changed yet:\n")?;
        write!(out, "{preview}")?;
    }
    out.flush()?;
    drop(out);

    // SY-05: a dry run never reaches sudo.
    if stop_for_dry_run(startup) || selected.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    let Some(sudo) = &tasks.sudo else {
        return Ok(ExitCode::SUCCESS);
    };
    // SY-06
    if !go_ahead(cli, "Run all of the above with sudo?")? {
        return Ok(ExitCode::SUCCESS);
    }
    // SY-11: fail closed before the first command.
    let mut log = molt::safe_delete::CommandLog::open(&startup.env.context(&startup.config))?;
    let result = system::act(sudo, &selected, root, search_path, &mut log)?;
    if cli.json {
        println!("{}", serde_json::json!({ "result": result }));
    } else {
        print!("{result}");
    }
    Ok(ExitCode::from(result.exit_code()))
}

/// CN-07, SY-05: a dry run stops after the preview, saying why.
fn stop_for_dry_run(startup: &Startup) -> bool {
    let Decision::DryRun(_) = startup.decision else {
        return false;
    };
    if let Some(notice) = startup::dry_run_notice(startup.decision) {
        eprintln!("molt: {notice}");
    }
    true
}

/// CN-05, SY-06: `--yes`, or a yes on the terminal. With neither, nothing changes.
fn go_ahead(cli: &Cli, question: &str) -> Result<bool, Box<dyn std::error::Error>> {
    if cli.yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(
            "no terminal to ask for confirmation; pass --yes to go ahead. Nothing changed".into(),
        );
    }
    let yes = confirm(question)?;
    if !yes {
        println!("Nothing changed.");
    }
    Ok(yes)
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
