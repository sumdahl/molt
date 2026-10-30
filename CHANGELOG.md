# Changelog

All notable changes to molt are recorded here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning: [SemVer](https://semver.org/).
The release workflow publishes the section matching the pushed tag as the GitHub Release notes.

## [Unreleased]

### Added
- Project foundation: principles spec (`specs/000`), safe-delete spec (`specs/001`), agent guide (`AGENTS.md`).
- `molt --version` / `molt --help`, smoke tests, CI, and tag-driven release workflow.
- MIT license.
- Global flags `--dry-run`, `--live`, `--yes` and `--json`. With `--json`, fatal errors are also printed to stdout as `{"error": "…"}`.
- Config file `~/.config/molt/config.toml` (`dry_run`, `whitelist`, `disabled_rules`, `purge_paths`) and user rule files in `~/.config/molt/rules/`. molt stops if either can't be read exactly.
- `MOLT_DRY_RUN=1` and `dry_run = true` make every run a dry run unless `--live` is given. `MOLT_NO_OPLOG=1` turns off the operation log.
- molt refuses to run as root.
