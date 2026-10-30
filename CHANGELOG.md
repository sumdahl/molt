# Changelog

All notable changes to molt are recorded here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning: [SemVer](https://semver.org/).
The release workflow publishes the section matching the pushed tag as the GitHub Release notes.

## [Unreleased]

### Added
- `molt analyze [PATH]`: where the space in a folder goes, largest first, with `--depth`, `--all` and `--json`. On-disk sizes, hard links counted once, never follows symlinks or crosses filesystems, `+` on sizes that are lower bounds (exit 3). Walks folders in parallel on the standard library's threads. On a terminal it opens a full-screen browser with Vim keys and arrows, written in molt with no terminal library; `--list` prints the listing instead. In the browser, `Space` marks items and `d` moves them to the trash, after the full safe-delete preview and a yes; there is no permanent delete. A dry run shows the preview only. The browser always restores the terminal. Spec: `specs/012-analyze.md`.

- `molt status`: CPU (count, how busy over 200 ms, load, uptime), memory and swap, each real disk once (no pseudo filesystems or repeats; a dead share is "unavailable" after 1 s), process counts with every zombie named, disk and network rates, temperatures (marked hot past the sensor's own limit), fans, battery charge and health, and a health score out of 100 that lists each deduction. Read-only from `/proc` and `/sys`, no other programs, `--json`, exit 0. `--watch` refreshes the report full screen every 2 s (`--interval` 1 to 60) with rates over the whole period, `p` to pause, `r` to refresh, `q` to quit; it needs a terminal and can't be combined with `--json`. Spec: `specs/013-status.md`.
- `molt uninstall <app>`: what an app left in your home (its XDG settings, data, state and cache folders, `~/.<app>`, and its menu and autostart entries), matched by exact name only. Shared folder names are refused, and a running app is left alone. The cache is deleted and the rest goes to the trash (`--permanent` to delete), after `clean`'s full preview and question. The package that owns the app (pacman, including AUR packages, apt, dnf or zypper; exact name or the owner of the program on `$PATH`) is shown in the same preview with everything its manager would remove, then removed first with `sudo -- <manager> remove` (`pacman -Rns`). The manager still asks for itself; if it fails or you say no, the leftovers are kept (exit 3). Flatpaks (by app id; `--user` ones without `sudo`) and snaps are removed the same way, with `~/.var/app/<id>` and `~/snap/<name>` moved to the trash; `<app>.AppImage` in `~/Applications` or `~/.local/bin` and its hicolor icons are leftovers too. Spec: `specs/014-uninstall.md`.

- `molt optimize`: packages nothing needs, through each manager's own command: pacman orphans (`-Qdtq`, removed with `-Rns` naming exactly the listed packages), `apt-get autoremove`, `dnf` and `zypper` unneeded packages. Queries run without root; the preview lists every package and the exact `sudo` command; the manager still asks for itself. Then upkeep: `fstrim --all` through `sudo`, `fc-cache` as you without `sudo`, and `resolvectl flush-caches` through `sudo` when systemd-resolved is running (its cache size in the preview). Last, read-only checks that never use `sudo`: failed systemd units (system and user) with where to look, and each real disk's SMART health (passed, FAILING, needs root, or no data). `--only`/`--skip`, `--json`, logged. Spec: `specs/015-optimize.md`.

- A green ASCII-art molt banner at the top of `molt --help` and a bare `molt`, on a terminal only. Never when piped, with `--json`, with `--version`, or above a named command. `NO_COLOR` or `TERM=dumb` prints it plain. Spec: `specs/016-banner.md`.

- `molt history`: what molt trashed, deleted and ran, newest first, read from the operation log without writing anything. Local times, sizes, exit statuses of failed commands, how many went to the trash. `--limit`, `--all`, `--since`, `--json`. An unreadable line is skipped and counted (exit 3). Spec: `specs/017-history.md`.

- `molt completions <bash|zsh|fish>`: a static completion script for every command, flag and fixed id, generated from molt's own command definitions with no completion crate; it never runs molt on Tab. Spec: `specs/018-completions.md`.

- The `molt` menu: a bare `molt` on a terminal lists every command under the banner; arrows or `j`/`k` and Enter run one exactly as typed (dry run, preview and question included), with the menu's `--dry-run`/`--live`/`--yes`, then return. Uninstall asks for the app's name; Analyze opens on `~`; Status runs live. Piped, a bare `molt` is still `molt scan`. Spec: `specs/019-menu.md`.

### Changed
- `system`, `optimize` and `purge` check `--only`/`--skip` ids through clap, which lists the valid ones and suggests the closest (still exit 2).
- Releases are signed: the release workflow builds only from a tag signed with a maintainer's SSH key, listed in `.github/allowed_signers`. Check a tag with `git -c gpg.ssh.allowedSignersFile=.github/allowed_signers verify-tag vX.Y.Z`.

## [0.1.0] - 2026-10-30

### Added
- Project foundation: principles spec (`specs/000`), safe-delete spec (`specs/001`), agent guide (`AGENTS.md`).
- `molt --version` / `molt --help`, smoke tests, CI, and tag-driven release workflow.
- MIT license.
- Global flags `--dry-run`, `--live`, `--yes` and `--json`. With `--json`, fatal errors are also printed to stdout as `{"error": "…"}`.
- Config file `~/.config/molt/config.toml` (`dry_run`, `whitelist`, `disabled_rules`, `purge_paths`) and user rule files in `~/.config/molt/rules/`. molt stops if either can't be read exactly.
- `MOLT_DRY_RUN=1` and `dry_run = true` make every run a dry run unless `--live` is given. `MOLT_NO_OPLOG=1` turns off the operation log.
- molt refuses to run as root.
- `molt scan` (also `molt` with no command): shows how much space could be freed, by category and rule, without changing anything. `--json` lists every path. The sizes come from the same dry run `clean` will use, so whitelisted and protected paths are never counted. Exit code 3 if some folder couldn't be read.
- 24 built-in rules for the caches that fill a developer's disk: npm, pnpm, Yarn, Bun, Cargo, Go, Gradle, Maven, uv, pip, mise, yay, paru, JetBrains, Firefox, Chrome, Chromium, Brave, Spotify, Discord, Slack and thumbnails. Each lists the tool's own cleanup command where there is one, and the programs that must not be running. Reviewed in `specs/010-built-in-rules.md`.
- Locked folders (yours, but unreadable, like the `d--x--x--x` folders makepkg leaves in AUR caches) no longer stop a scan. `molt scan` shows them per rule, measures everything around them, and never changes their permissions. `molt clean` will ask before opening them.
- `molt clean`: frees what `scan` finds. It prints the full preview first (also with `--yes` and `--json`), asks once with no as the default, runs each tool's own cleanup command instead of deleting its files, leaves out caches whose program is running, skips anything changed since the preview, and asks separately before opening locked folders (or `--fix-permissions`). `--only`/`--skip` pick rules; `--trash`/`--permanent` override the mode. Tool commands are logged too.
- `molt purge`: removes project build folders (Cargo, Node, Python venvs, Gradle, Maven, Next.js, Zig) under `purge_paths`, only next to their project file, never in projects changed in the last 7 days or holding Git-tracked files. Same preview, question and safe delete as `clean`; `--only`/`--skip` pick kinds, `--trash` sets them aside. Spec: `specs/008-purge.md`.
- `molt installer`: moves old installers (`.deb`, `.rpm`, `.pkg.tar.*`, `.AppImage`, `.iso`, `.run`) in `~/Downloads` and `~/Desktop` to the trash, leaving files from the last 14 days, folders and symlinks. `--permanent` deletes instead; `--only`/`--skip` pick rules. `scan` and `clean` never include them. Spec: `specs/009-installer.md`.
- `molt system`: the pacman cache, the systemd journal and old coredumps, cleaned by `paccache`, `journalctl` and `systemd-tmpfiles` through `sudo`. It previews each task without `sudo` first, shows the exact commands, and asks once. `--only`/`--skip` pick tasks; every command is logged. Spec: `specs/007-system.md`.
