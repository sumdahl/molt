#![forbid(unsafe_code)]

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::SystemTime;

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use onyx::analyze;
use onyx::clean::{self, Busy};
use onyx::completions::{self, Shell};
use onyx::config::{Confirm, Decision, Requested};
use onyx::fingerprint::{self, Verified};
use onyx::install;
use onyx::installer;
use onyx::optimize;
use onyx::processes;
use onyx::purge;
use onyx::rules::{Rule, RuleId};
use onyx::safe_delete::{
    self, Candidate, CommandLog, Confirmed, Execution, Mode, Permissions, Plan, Privilege,
};
use onyx::scan::{self, Survey};
use onyx::startup::{self, Startup};
use onyx::system;
use onyx::uninstall::package::{self, PackagePreview};
use onyx::uninstall::{self, AppName};

#[derive(Parser)]
// PK-07: the crate is `onyx-cli`; the command is `onyx`. The about is written here,
// not taken from the crates.io description.
#[command(name = "onyx", version, about = "Reclaim disk space on Linux, safely")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Show what would happen without changing anything.
    #[arg(long, global = true, conflicts_with = "live")]
    dry_run: bool,
    /// Act, even when the config or ONYX_DRY_RUN asks for a dry run.
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
    /// Show how much space onyx could free, without changing anything (the default).
    Scan,
    /// Free the space `scan` shows: preview first, then ask, then clean.
    Clean(CleanArgs),
    /// Remove build folders (target, node_modules, .venv…) of projects under purge_paths:
    /// preview first, then ask.
    Purge(PurgeArgs),
    /// Move old installers (.deb, .rpm, .AppImage, .iso…) in Downloads and Desktop to the
    /// trash: preview first, then ask.
    Installer(InstallerArgs),
    /// Remove an app and what it left in your home (settings, data, cache, menu
    /// entries), its package through its own manager: preview first, then ask.
    Uninstall(UninstallArgs),
    /// Show what takes the space in a folder, largest first: a browser on a terminal, a
    /// listing otherwise. Changes nothing.
    Analyze(AnalyzeArgs),
    /// Show the machine's health: CPU, memory, disks, sensors and a score. Changes nothing.
    Status(StatusArgs),
    /// Free system space (pacman cache, journal, coredumps) through each tool's own
    /// command, run with sudo: preview first, then ask.
    System(SystemArgs),
    /// Remove packages nothing needs (orphans, autoremove) through each package manager,
    /// run with sudo: preview first, then ask.
    Optimize(OptimizeArgs),
    /// Show what onyx has removed and run, newest first. Changes nothing.
    History(HistoryArgs),
    /// Print a shell completion script: bash, zsh or fish.
    Completions(CompletionsArgs),
    /// Replace onyx with the newest release the maintainer signed: preview first, then
    /// ask. Only a copy in your home; a package manager updates its own.
    Update,
    /// Take onyx off this machine (binary, config, cache, state): preview first, then
    /// ask. The operation log stays unless --with-log.
    Remove(RemoveArgs),
}

#[derive(Clone, Args)]
struct RemoveArgs {
    /// Remove the operation log too.
    #[arg(long)]
    with_log: bool,
    /// Delete permanently instead of moving to the trash.
    #[arg(long)]
    permanent: bool,
}

#[derive(Clone, Args)]
struct CompletionsArgs {
    shell: Shell,
}

/// CO-03: a fixed list of ids, which clap checks and completions offer.
fn ids<const N: usize>(ids: [&'static str; N]) -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(ids)
}

#[derive(Clone, Args)]
struct PurgeArgs {
    /// Only these kinds of project.
    #[arg(long, num_args = 1.., value_name = "KIND", value_parser = ids(purge::ProjectKind::ALL.map(purge::ProjectKind::id)))]
    only: Vec<String>,
    /// Leave these kinds out.
    #[arg(long, num_args = 1.., value_name = "KIND", value_parser = ids(purge::ProjectKind::ALL.map(purge::ProjectKind::id)))]
    skip: Vec<String>,
    /// Move to the trash instead of deleting.
    #[arg(long)]
    trash: bool,
}

#[derive(Clone, Args)]
struct InstallerArgs {
    /// Only these rules (ids as in `onyx installer --json`).
    #[arg(long, num_args = 1.., value_name = "ID")]
    only: Vec<String>,
    /// Leave these rules out.
    #[arg(long, num_args = 1.., value_name = "ID")]
    skip: Vec<String>,
    /// Delete permanently instead of moving to the trash.
    #[arg(long)]
    permanent: bool,
}

#[derive(Clone, Args)]
struct UninstallArgs {
    /// The app's folder name, exactly as in ~/.config (case matters).
    app: String,
    /// Delete permanently instead of moving to the trash.
    #[arg(long)]
    permanent: bool,
}

#[derive(Clone, Args)]
struct AnalyzeArgs {
    /// The folder to measure (default: the current directory).
    path: Option<PathBuf>,
    /// How many levels to list.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    depth: u32,
    /// List every entry, not just the 20 largest per level.
    #[arg(long)]
    all: bool,
    /// Print the listing even on a terminal, instead of opening the browser.
    #[arg(long)]
    list: bool,
}

#[derive(Clone, Args)]
struct StatusArgs {
    /// Keep the report on screen, refreshed, until q.
    #[arg(long)]
    watch: bool,
    /// Seconds between refreshes with --watch.
    #[arg(
        long,
        requires = "watch",
        default_value_t = 2,
        value_parser = clap::value_parser!(u64).range(1..=60)
    )]
    interval: u64,
}

#[derive(Clone, Args)]
struct SystemArgs {
    /// Run only these tasks.
    #[arg(long, num_args = 1.., value_name = "ID", value_parser = ids(system::TaskId::ALL.map(system::TaskId::id)))]
    only: Vec<String>,
    /// Leave these tasks out.
    #[arg(long, num_args = 1.., value_name = "ID", value_parser = ids(system::TaskId::ALL.map(system::TaskId::id)))]
    skip: Vec<String>,
}

#[derive(Clone, Args)]
struct OptimizeArgs {
    /// Run only these tasks.
    #[arg(long, num_args = 1.., value_name = "ID", value_parser = ids(optimize::TaskId::ALL.map(optimize::TaskId::id)))]
    only: Vec<String>,
    /// Leave these tasks out.
    #[arg(long, num_args = 1.., value_name = "ID", value_parser = ids(optimize::TaskId::ALL.map(optimize::TaskId::id)))]
    skip: Vec<String>,
}

#[derive(Clone, Args)]
struct HistoryArgs {
    /// Show at most this many entries.
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..))]
    limit: u32,
    /// Show every entry.
    #[arg(long, conflicts_with = "limit")]
    all: bool,
    /// Only entries from this day on (YYYY-MM-DD, local time).
    #[arg(long, value_name = "DATE", value_parser = since_date)]
    since: Option<jiff::Timestamp>,
}

/// HI-04: local midnight at the start of `date`.
fn since_date(date: &str) -> Result<jiff::Timestamp, String> {
    let day: jiff::civil::Date = date
        .parse()
        .map_err(|_| format!("{date:?} isn't a date like 2026-10-30"))?;
    day.to_zoned(jiff::tz::TimeZone::system())
        .map(|midnight| midnight.timestamp())
        .map_err(|error| error.to_string())
}

#[derive(Clone, Args)]
struct CleanArgs {
    /// Clean only these rules (ids as in `onyx scan --json`).
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
    // BN-02: the banner tops the main help on a terminal.
    let mut command = Cli::command();
    if let Some(banner) = onyx::banner::for_terminal() {
        // clap puts its own blank line after it.
        command = command.before_help(banner.trim_end().to_owned());
    }
    // P-13: clap exits 2 on bad usage, and 0 for --help and --version.
    let cli = match Cli::from_arg_matches(&command.get_matches()) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    match run(&cli) {
        Ok(code) => code,
        Err(error) => {
            // CL-03, CL-09
            eprintln!("onyx: {error}");
            if cli.json {
                println!("{}", serde_json::json!({ "error": error.to_string() }));
            }
            ExitCode::from(1)
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode, Box<dyn std::error::Error>> {
    // CO-01: before startup, so even a broken config completes.
    if let Some(Command::Completions(args)) = &cli.command {
        print!("{}", completions::script(args.shell, Cli::command()));
        return Ok(ExitCode::SUCCESS);
    }
    // MN-01: a bare `onyx` with a terminal on both ends is the menu.
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if cli.command.is_none() && !cli.json && terminal {
        return run_menu(cli);
    }
    // BN-02, BN-03: a bare `onyx` only, never above a named command or JSON.
    if cli.command.is_none() && !cli.json {
        if let Some(banner) = onyx::banner::for_terminal() {
            print!("{banner}");
        }
    }
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
        Some(Command::Installer(args)) => run_installer(cli, args, &startup),
        Some(Command::Uninstall(args)) => run_uninstall(cli, args, &startup),
        Some(Command::System(args)) => run_system(cli, args, &startup),
        Some(Command::Optimize(args)) => run_optimize(cli, args, &startup),
        // HI-01: read-only, so a dry run is the same run.
        Some(Command::History(args)) => run_history(cli, args, &startup),
        Some(Command::Update) => run_update(cli, &startup),
        Some(Command::Remove(args)) => run_remove(cli, args, &startup),
        // Handled before startup (CO-01).
        Some(Command::Completions(_)) => Ok(ExitCode::SUCCESS),
        // AN-09: read-only, so a dry run is the same run.
        Some(Command::Analyze(args)) => run_analyze(cli, args, &startup),
        // ST-01, ST-08: read-only, and always 0.
        Some(Command::Status(args)) if args.watch => run_watch(cli, args),
        Some(Command::Status(_)) => {
            let status = onyx::status::status();
            if cli.json {
                println!("{}", serde_json::to_string(&status)?);
            } else {
                print!("{status}");
            }
            Ok(ExitCode::SUCCESS)
        }
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
            eprintln!("onyx: no enabled rule has the id `{unknown}`; nothing changed");
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
            eprintln!("onyx: there is no project kind `{unknown}`; nothing changed");
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
            "onyx: no purge_paths set. Add the folders holding your projects to {}, \
             e.g. purge_paths = [\"~/projects\"]",
            startup.env.dirs.config.join("onyx/config.toml").display()
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
    preview.report.freed_by = "onyx purge".to_owned();
    print_preview(cli, &preview, &[])?;
    if !cli.json {
        print_left_out(
            &preview,
            "build folders",
            &[
                ("recently-changed", "changed in the last 7 days"),
                ("git-tracked", "may hold Git-tracked files"),
            ],
        );
    }
    ask_and_act(cli, startup, &preview, &kinds, false)
}

/// IN-01..IN-05: old installers in Downloads and Desktop, then on as `clean` does.
fn run_installer(
    cli: &Cli,
    args: &InstallerArgs,
    startup: &Startup,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let rules = match clean::select(&startup.installers, &args.only, &args.skip) {
        Ok(rules) => rules,
        Err(unknown) => {
            eprintln!("onyx: no installer rule has the id `{unknown}`; nothing changed");
            return Ok(ExitCode::from(2));
        }
    };
    let env = &startup.env;
    let mut findings = onyx::rules::find(&rules, &env.dirs, &env.search_path);
    installer::keep_old_files(&mut findings, SystemTime::now());
    // IN-04
    let mode = args.permanent.then_some(Mode::Permanent);
    let mut preview = scan::measure_findings(&rules, findings, startup, mode)?;
    preview.report.freed_by = "onyx installer".to_owned();
    print_preview(cli, &preview, &[])?;
    if !cli.json {
        print_left_out(
            &preview,
            "installers",
            &[("recently-changed", "changed in the last 14 days")],
        );
    }
    ask_and_act(cli, startup, &preview, &rules, false)
}

/// UN-01 to UN-07: an app's leftovers in the home, then on as `clean` does.
fn run_uninstall(
    cli: &Cli,
    args: &UninstallArgs,
    startup: &Startup,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let app = match AppName::try_from(args.app.as_str()) {
        Ok(app) => app,
        Err(why) => {
            eprintln!("onyx: {why}; nothing changed");
            return Ok(ExitCode::from(2));
        }
    };
    let search_path = &startup.env.search_path;
    let (rules, busy) = clean::leave_out_busy(uninstall::rules(&app)?, &processes::running());
    // UN-14: a running app keeps its package too.
    let package = busy
        .is_empty()
        .then(|| package::find(&app, search_path))
        .flatten();
    let sudo = onyx::rules::find_tool("sudo", search_path);
    let package_preview = package
        .as_ref()
        .map(|package| package::preview(package, sudo.as_deref(), search_path));
    // UN-05
    let mode = args.permanent.then_some(Mode::Permanent);
    let mut preview = scan::survey(&rules, startup, mode)?;
    preview.report.freed_by = format!("onyx uninstall {app}");
    print_uninstall_preview(cli, &app, &preview, &busy, package_preview.as_ref())?;

    // UN-10: a dry run never reaches sudo.
    let removal = package
        .as_ref()
        .and_then(|package| package.command(sudo.as_deref()));
    if stop_for_dry_run(startup) || (removal.is_none() && preview.report.rules.is_empty()) {
        return Ok(ExitCode::SUCCESS);
    }
    // UN-11: one question for both.
    if !go_ahead(cli, startup, "Remove all of the above?")? {
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(removal) = &removal {
        // SY-11: fail closed before the command.
        let mut log = CommandLog::open(&startup.env.context(&startup.config))?;
        let run = package::remove(removal, search_path, &mut log)?;
        if cli.json {
            println!("{}", serde_json::json!({ "package": run }));
        }
        if run.exit != Some(0) {
            // UN-12: the app is still installed, so its settings stay.
            if !cli.json {
                println!("`{}` failed, so the leftovers were kept.", run.command);
            }
            return Ok(ExitCode::from(3));
        }
    }
    if preview.report.rules.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    act_on(cli, startup, &preview, &rules, false)
}

/// UN-04, UN-07, UN-10: the package above the leftovers, all before anything changes.
fn print_uninstall_preview(
    cli: &Cli,
    app: &AppName,
    preview: &Survey,
    busy: &[Busy],
    package: Option<&PackagePreview>,
) -> Result<(), Box<dyn std::error::Error>> {
    if cli.json {
        let preview = serde_json::json!({
            "preview": { "report": preview.report, "busy": busy, "package": package }
        });
        println!("{preview}");
        return Ok(());
    }
    println!("Preview. Nothing has changed yet:\n");
    if let Some(package) = package {
        print!("{package}");
    }
    print!("{}", preview.report);
    if !busy.is_empty() {
        println!("{app} is running: quit {app} first, then run this again.");
    } else if preview.report.rules.is_empty() {
        println!("Nothing of {app} in your home.");
    }
    if package.is_none() {
        println!("{app} itself isn't removed: no package manager here owns it.");
    }
    Ok(())
}

/// PG-04, PG-05, IN-03: how many items the preview left out, and why. `reasons` pairs a
/// skip reason with what to say about it.
fn print_left_out(preview: &Survey, what: &str, reasons: &[(&str, &str)]) {
    for (reason, why) in reasons {
        let count = preview
            .report
            .skipped
            .iter()
            .filter(|skip| skip.reason == *reason)
            .count();
        if count > 0 {
            println!("Left out {count} {what}: {why}. See --json.");
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
    if !go_ahead(cli, startup, "Clean all of the above?")? {
        return Ok(ExitCode::SUCCESS);
    }
    act_on(cli, startup, preview, rules, fix_permissions)
}

/// CN-06 to CN-11: cleans what the preview showed, once the user said yes.
fn act_on(
    cli: &Cli,
    startup: &Startup,
    preview: &Survey,
    rules: &[Rule],
    fix_permissions: bool,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
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

/// AN-01..AN-09
fn run_analyze(
    cli: &Cli,
    args: &AnalyzeArgs,
    startup: &Startup,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let path = match &args.path {
        Some(path) => path.clone(),
        None => std::env::current_dir()?,
    };
    // AN-10
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if terminal && !cli.json && !args.list {
        // P-05: in a dry run the trash step only previews.
        let dry_run = match startup.decision {
            Decision::DryRun(_) => Some(format!(
                "Dry run, so nothing moved{}.",
                startup::dry_run_notice(startup.decision)
                    .map(|notice| format!(" ({notice})"))
                    .unwrap_or_default()
            )),
            Decision::Live => None,
        };
        analyze::browse(
            &path,
            &startup.env.context(&startup.config),
            dry_run.as_deref(),
        )?;
        return Ok(ExitCode::SUCCESS);
    }
    let depth = usize::try_from(args.depth)?;
    let tree = analyze::measure(&path, depth)?;
    if cli.json {
        println!("{}", serde_json::to_string(&tree)?);
    } else {
        print!("{}", analyze::Listing::new(&tree, depth, args.all));
    }
    // AN-03, P-13
    Ok(if tree.partial {
        ExitCode::from(3)
    } else {
        ExitCode::SUCCESS
    })
}

/// ST-15 to ST-18: the live view needs a terminal, and isn't for scripts.
fn run_watch(cli: &Cli, args: &StatusArgs) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let refused = match (cli.json, terminal) {
        (true, _) => Some("--watch can't be used with --json; poll `onyx status --json` instead"),
        (_, false) => Some("--watch needs a terminal; use `onyx status` or `--json` instead"),
        _ => None,
    };
    if let Some(why) = refused {
        eprintln!("onyx: {why}");
        return Ok(ExitCode::from(2));
    }
    onyx::status::watch(std::time::Duration::from_secs(args.interval))?;
    Ok(ExitCode::SUCCESS)
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
            eprintln!("onyx: there is no system task `{unknown}`; nothing changed");
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
    if !go_ahead(cli, startup, "Run all of the above with sudo?")? {
        return Ok(ExitCode::SUCCESS);
    }
    // SY-11: fail closed before the first command.
    let mut log = CommandLog::open(&startup.env.context(&startup.config))?;
    let result = system::act(sudo, &selected, root, search_path, &mut log)?;
    if cli.json {
        println!("{}", serde_json::json!({ "result": result }));
    } else {
        print!("{result}");
    }
    Ok(ExitCode::from(result.exit_code()))
}

/// A plan over `paths` in `mode`, run as `execution`.
fn self_plan(paths: &[PathBuf], mode: Mode, execution: Execution) -> Plan {
    Plan {
        candidates: paths
            .iter()
            .filter_map(|path| Candidate::scan(path).ok())
            .collect(),
        mode,
        execution,
        privilege: Privilege::User,
        permissions: Permissions::Keep,
    }
}

/// SE-06: onyx's own download folder, removed whatever happened and never logged.
fn remove_scratch(folder: &std::path::Path, ctx: &onyx::safe_delete::Context) {
    let mut quiet = ctx.clone();
    quiet.oplog = None;
    let plan = self_plan(&[folder.to_path_buf()], Mode::Permanent, Execution::Live);
    if safe_delete::execute(&plan, &quiet).is_err() {
        eprintln!("onyx: couldn't remove {}", folder.display());
    }
}

/// SE-01 to SE-06: the newest signed release, checked, previewed, then swapped in.
fn run_update(cli: &Cli, startup: &Startup) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let ctx = startup.env.context(&startup.config);
    let current = env!("CARGO_PKG_VERSION");
    // SE-04: decided before anything is fetched.
    let binary = match install::own_binary(&startup.env) {
        Ok(binary) => binary,
        Err(why) => return Err(format!("{why}; nothing changed").into()),
    };
    let tools = install::Tools::find(&startup.env.search_path)?;
    let tag = tools.latest()?;
    if !install::is_newer(&tag, current) {
        println!("onyx {current} is the newest release. Nothing changed.");
        return Ok(ExitCode::SUCCESS);
    }
    let prepared = install::prepare(&tools, &tag)?;
    let result = update_from(cli, startup, &ctx, &binary, current, &prepared);
    remove_scratch(&prepared.folder, &ctx);
    result
}

fn update_from(
    cli: &Cli,
    startup: &Startup,
    ctx: &onyx::safe_delete::Context,
    binary: &std::path::Path,
    current: &str,
    prepared: &install::Prepared,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    // SE-03
    println!("Preview. Nothing has changed yet:\n");
    println!(
        "  onyx {current} -> {}",
        prepared.tag.trim_start_matches('v')
    );
    println!("  from {}", prepared.url);
    for step in &prepared.steps {
        let mark = if step.ok { "ok    " } else { "FAILED" };
        println!("  {mark} {}", step.what);
    }
    let Some(new) = &prepared.binary else {
        return Err("the release didn't pass its checks; nothing changed".into());
    };
    println!(
        "  would replace {} (the old one goes to the trash)\n",
        binary.display()
    );
    if stop_for_dry_run(startup) || !go_ahead(cli, startup, "Replace onyx?")? {
        return Ok(ExitCode::SUCCESS);
    }
    // SY-11: fail closed before the swap.
    let mut log = CommandLog::open(ctx)?;
    let old = safe_delete::swap_in(binary, new, ctx.euid).map_err(|errno| {
        format!("couldn't put the new onyx in place ({errno}); nothing changed")
    })?;
    let version = prepared.tag.trim_start_matches('v');
    log.record(
        &[
            "onyx".into(),
            "update".into(),
            "--to".into(),
            version.into(),
        ],
        Some(0),
        0,
    )?;
    let report = safe_delete::execute(&self_plan(&[old], Mode::Trash, Execution::Live), ctx)?;
    for (path, outcome) in &report.results {
        println!("{}", install::outcome_line(path, outcome));
    }
    println!("onyx is now {version}.");
    Ok(ExitCode::from(
        u8::try_from(report.exit_code()).unwrap_or(3),
    ))
}

/// SE-07, SE-08: onyx's own files, after the preview and a yes.
fn run_remove(
    cli: &Cli,
    args: &RemoveArgs,
    startup: &Startup,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let ctx = startup.env.context(&startup.config);
    let binary = install::own_binary(&startup.env);
    let paths = install::leftovers(binary.as_deref().ok(), &startup.env.dirs, args.with_log);
    let mode = if args.permanent {
        Mode::Permanent
    } else {
        Mode::Trash
    };
    let preview = safe_delete::execute(&self_plan(&paths, mode, Execution::DryRun), &ctx)?;
    println!("Preview. Nothing has changed yet:\n");
    for (path, outcome) in &preview.results {
        println!("{}", install::outcome_line(path, outcome));
    }
    if let Err(why) = &binary {
        println!("\nThe binary stays: {why}.");
    }
    if !args.with_log {
        println!("The operation log stays (--with-log removes it).");
    }
    if preview.results.is_empty()
        || stop_for_dry_run(startup)
        || !go_ahead(cli, startup, "Remove onyx?")?
    {
        return Ok(ExitCode::SUCCESS);
    }
    let report = safe_delete::execute(&self_plan(&paths, mode, Execution::Live), &ctx)?;
    for (path, outcome) in &report.results {
        println!("{}", install::outcome_line(path, outcome));
    }
    println!("onyx is removed.");
    Ok(ExitCode::from(
        u8::try_from(report.exit_code()).unwrap_or(3),
    ))
}

/// MN-02 to MN-04: the menu, until it's quit. Each pick is parsed and run like a
/// typed command line, with the menu's global flags.
fn run_menu(cli: &Cli) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let banner = onyx::banner::for_terminal().unwrap_or_default();
    let mut selected = 0;
    while let Some(entry) = onyx::menu::pick(&banner, &mut selected)? {
        let mut args: Vec<std::ffi::OsString> = vec!["onyx".into()];
        for (on, flag) in [
            (cli.dry_run, "--dry-run"),
            (cli.live, "--live"),
            (cli.yes, "--yes"),
        ] {
            if on {
                args.push(flag.into());
            }
        }
        args.extend(entry.args().iter().map(Into::into));
        match entry {
            onyx::menu::Entry::Uninstall => match ask_app()? {
                Some(app) => args.push(app.into()),
                None => continue,
            },
            onyx::menu::Entry::Analyze => args.extend(std::env::var_os("HOME")),
            _ => {}
        }
        let picked = Cli::try_parse_from(&args)?;
        let code = run(&picked).unwrap_or_else(|error| {
            eprintln!("onyx: {error}");
            ExitCode::from(1)
        });
        // MN-04: ExitCode can't be read back, and onyx only uses 0 to 3.
        if let Some(n) = (1..=3).find(|n| code == ExitCode::from(*n)) {
            println!("\nExit code {n}.");
        }
        print!("\nPress Enter to go back to the menu. ");
        std::io::stdout().flush()?;
        std::io::stdin().lock().read_line(&mut String::new())?;
    }
    Ok(ExitCode::SUCCESS)
}

/// MN-03: the app to uninstall, asked on a normal line. `None` when left empty.
fn ask_app() -> std::io::Result<Option<String>> {
    print!("App to uninstall (its exact folder name; empty to go back): ");
    std::io::stdout().flush()?;
    let mut name = String::new();
    std::io::stdin().lock().read_line(&mut name)?;
    let name = name.trim();
    Ok((!name.is_empty()).then(|| name.to_owned()))
}

/// HI-01 to HI-07: the operation log, newest first.
fn run_history(
    cli: &Cli,
    args: &HistoryArgs,
    startup: &Startup,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    // The path the log is written to, even when ONYX_NO_OPLOG stops new entries.
    let log = startup.env.dirs.state.join("onyx/operations.log");
    let Some(history) = onyx::history::read(&log)? else {
        if cli.json {
            println!(
                "{}",
                serde_json::json!({ "entries": [], "skipped_lines": 0, "left_out": 0 })
            );
        } else {
            println!("Nothing in the history yet.");
        }
        return Ok(ExitCode::SUCCESS);
    };
    let limit = (!args.all).then_some(args.limit as usize);
    let shown = history.select(args.since, limit);
    if cli.json {
        println!("{}", serde_json::to_string(&shown)?);
    } else {
        print!("{shown}");
    }
    Ok(ExitCode::from(shown.exit_code()))
}

/// OP-01 to OP-08: like `onyx system`, the preview, one question, then each command
/// through sudo.
fn run_optimize(
    cli: &Cli,
    args: &OptimizeArgs,
    startup: &Startup,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let search_path = &startup.env.search_path;
    let tasks = optimize::detect(search_path);
    let selected = match optimize::select(tasks.active.clone(), &args.only, &args.skip) {
        Ok(selected) => selected,
        Err(unknown) => {
            eprintln!("onyx: there is no optimize task `{unknown}`; nothing changed");
            return Ok(ExitCode::from(2));
        }
    };
    let preview = optimize::preview(&tasks, &selected, search_path);
    if cli.json {
        println!("{}", serde_json::json!({ "preview": preview }));
    } else {
        println!("Preview. Nothing has changed yet:\n");
        print!("{preview}");
    }
    // OP-07: a dry run never reaches sudo.
    if stop_for_dry_run(startup) || !preview.has_work() {
        return Ok(ExitCode::SUCCESS);
    }
    if !go_ahead(cli, startup, "Run all of the above with sudo?")? {
        return Ok(ExitCode::SUCCESS);
    }
    // SY-11: fail closed before the first command.
    let mut log = CommandLog::open(&startup.env.context(&startup.config))?;
    let result = optimize::act(&preview, search_path, &mut log)?;
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
        eprintln!("onyx: {notice}");
    }
    true
}

/// CN-05, SY-06: `--yes`, or a yes on the terminal. With neither, nothing changes.
fn go_ahead(
    cli: &Cli,
    startup: &Startup,
    question: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let how = if cli.yes {
        Confirmed::YesFlag
    } else if !std::io::stdin().is_terminal() {
        return Err(
            "no terminal to ask for confirmation; pass --yes to go ahead. Nothing changed".into(),
        );
    } else {
        match ask(startup, question)? {
            Some(how) => how,
            None => {
                println!("Nothing changed.");
                return Ok(false);
            }
        }
    };
    // FP-06: before anything changes; a failed write stops the run (SD-15).
    let ctx = startup.env.context(&startup.config);
    CommandLog::open(&ctx)?
        .confirmed(how, question)
        .map_err(|error| format!("can't write the operation log: {error}. Nothing changed"))?;
    Ok(true)
}

/// FP-03, FP-04: the fingerprint when the config asks for it, else (or when it isn't
/// confirmed) the typed question. `None` is no.
fn ask(startup: &Startup, question: &str) -> std::io::Result<Option<Confirmed>> {
    if startup.config.confirm == Confirm::Fingerprint {
        eprintln!("{question}");
        eprintln!("Touch the fingerprint sensor to go ahead, or press Ctrl-C to stop.");
        match fingerprint::verify(&startup.env.search_path) {
            Verified::Matched => return Ok(Some(Confirmed::Fingerprint)),
            other => eprintln!("{other}; answer here instead."),
        }
    }
    Ok(confirm(question)?.then_some(Confirmed::Typed))
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
