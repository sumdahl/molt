# Changelog

All notable changes to molt are recorded here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning: [SemVer](https://semver.org/).
The release workflow publishes the section matching the pushed tag as the GitHub Release notes.

## [Unreleased]

### Added
- `molt analyze [PATH]`: where the space in a folder goes, largest first, with `--depth`, `--all` and `--json`. On-disk sizes, hard links counted once, never follows symlinks or crosses filesystems, `+` on sizes that are lower bounds (exit 3). Walks folders in parallel on the standard library's threads. On a terminal it opens a full-screen browser with Vim keys and arrows, written in molt with no terminal library; `--list` prints the listing instead. The browser changes nothing, and always restores the terminal. Spec: `specs/012-analyze.md`.

### Changed
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
