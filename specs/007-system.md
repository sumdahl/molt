# 007: `molt system`

Status: Accepted
Prefix: SY

## Goal

`molt system` frees space the user's own files can't reach: the pacman package cache, the systemd journal, and old coredumps. It is the one explicit privileged step that P-04 allows. It names every command it would run, previews each one without privileges, asks, and only then runs the commands through `sudo`. molt itself never runs as root.

## Non-goals

- Deleting system files itself. Every task is an owning tool's command (P-07); molt runs no safe-delete plan as root, and `Privilege::Root` stays unused.
- Docker, Flatpak and Snap. They are separate tasks for later, with their own review.
- Other package managers (apt, dnf, zypper). Their tasks come later; this spec defines the shape they fit into.
- Running as root, or keeping a `sudo` session alive between commands. `sudo`'s own timestamp handles repeat prompts.
- Config for the limits below. The defaults are fixed until someone needs them changed.

## Model

1. Start up (004). Root is still refused (CL-07): the user runs `molt system`, and molt runs `sudo` for each command.
2. **Detect** (P-10): each task is active only when its tools are found on the absolute `$PATH` entries (SC-10), as RE-08 does for rules.
3. **Preview** (P-14): for each active task, run its read-only preview command, without `sudo`, and show the task, its current size, the preview's summary, and the exact command a live run would run.
4. **Ask** (P-06): one yes/no for the whole list, default no.
5. **Act**: run each task's command as `sudo -- <command>`, in order, and measure the size after.
6. **Report** what each task freed, and the total.

## Tasks

| id | Active when found | Size shown | Preview (no sudo) | Live command |
|---|---|---|---|---|
| `pacman-cache` | `paccache` | on-disk size of `/var/cache/pacman/pkg` | `paccache -dk2`, `paccache -duk0` | `paccache -rk2`, `paccache -ruk0` |
| `journal` | `journalctl` | `journalctl --disk-usage` | none; shows the size and the limit | `journalctl --vacuum-size=500M` |
| `coredumps` | `systemd-tmpfiles`, and `/var/lib/systemd/coredump` exists | on-disk size of `/var/lib/systemd/coredump` | `systemd-tmpfiles --clean --dry-run --prefix /var/lib/systemd/coredump` (systemd 256 or later; older versions show the size only) | `systemd-tmpfiles --clean --prefix /var/lib/systemd/coredump` |

- `paccache -rk2` keeps the two newest versions of each installed package (a downgrade stays possible). `-ruk0` removes every cached version of packages that are no longer installed.
- The journal is trimmed to 500 MiB, oldest first. Nothing newer than the limit is lost.
- `systemd-tmpfiles --clean --prefix` applies only the system's own age rule for coredumps (`2w` on Arch, 3 days upstream), and nothing outside that folder.

## Requirements

### Choosing
- **SY-01** `molt system` lists every active task. `--only <id>…` and `--skip <id>…` narrow the list; an unknown id is a usage error (exit 2), as in CN-02.
- **SY-02** A task whose tools aren't found is left out and reported as inactive (P-10). A tool is found only on an absolute `$PATH` entry (SC-10), and `sudo` must be found the same way, or every task is inactive.

### Preview and consent
- **SY-03** Every run, live or dry, shows the preview first (P-14): each task's id, its current size, its preview's summary line, and the exact live command line, with `sudo` in front.
- **SY-04** Preview commands run without `sudo`, with no shell, stdin closed, and `$PATH` limited to its absolute entries. Their output goes to stderr. A preview that fails or can't start is reported, and its task is still shown with its current size.
- **SY-05** A dry run (`--dry-run`, the config, or `MOLT_DRY_RUN`, with the CL-06 notice) stops after the preview. It never runs `sudo` and never prompts for a password (P-05).
- **SY-06** A live run asks once, default no, as CN-05 does: anything but `y` or `yes` changes nothing and exits 0 with "Nothing changed." Without a terminal on stdin it needs `--yes`, or it prints the preview and exits 1.

### Acting
- **SY-07** Each live command runs as `sudo -- <tool path> <args>`, with the tool's absolute path, so `sudo` runs exactly the program the preview showed. No shell, stdin from the terminal (for `sudo`'s password prompt), and output on stderr.
- **SY-08** Tasks run in the order of the table. A task whose command fails (non-zero exit, or it can't start) is reported as failed with its exit status, and the remaining tasks still run: they are independent.
- **SY-09** What a task freed is its size before minus after, measured the same way as the preview.

### Reporting
- **SY-10** The result lists, per task, the commands run, their exit status, and the bytes freed, then the total. The exit code is 0 when every command succeeded, 3 when any failed (P-13). `--json` prints the preview and the result as in CN-04.
- **SY-11** Every command run through `sudo` is in the operation log (SD-15) as a command entry: its full command line, exit status and bytes freed. A log that can't be opened stops the run before the first command (fail closed).

## Test strategy

- Smoke tests on a temp `$PATH` with fake `paccache`, `journalctl`, `systemd-tmpfiles` and `sudo` scripts that record their arguments to a file and never touch the system. The fake `sudo` records its arguments and runs the rest.
- SY-05 / P-05: with `--dry-run`, the fake `sudo` records nothing, and only preview arguments are recorded.
- SY-07: the recorded `sudo` arguments start with `--` and an absolute tool path.
- SY-02: with no `paccache` on `$PATH`, the pacman task is inactive, and a relative `$PATH` entry holding a fake tool is ignored.
- SY-08: a fake tool that exits 1 makes its task failed, the next task still runs, and molt exits 3.
- The size of `/var/...` folders comes from a path that tests point at a temp folder (a test-build override, as the sandbox tripwire does), so no test reads the real system.

## Decisions

- **A separate command.** `sudo` never appears in `clean`.
- **pacman keeps two versions** of installed packages (`-rk2`) and drops uninstalled ones (`-ruk0`).
- **The journal is trimmed to 500 MiB.**
- **Coredumps follow the system's own age rule** through `systemd-tmpfiles`. molt decides nothing about them and deletes nothing as root.
