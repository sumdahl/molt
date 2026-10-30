# 003: Config and whitelist

Status: Implemented
Prefix: CF

## Goal

One small TOML file where the user says what onyx must never touch, which built-in rules to switch off, where their projects live, and whether onyx should only ever preview unless told otherwise. If onyx can't read that file exactly, it does nothing (P-03).

## Non-goals

- Writing the config. onyx only reads it; there is no `onyx config set`.
- Rule definitions. Those are rule files (002).
- What `purge` does with its roots. That is 008; this spec only reads them.

## Model

`$XDG_CONFIG_HOME/onyx/config.toml`:

```toml
dry_run = true                       # preview only, unless --live (CF-07)
whitelist = ["~/work/keep-this-build", "/data/vms"]
disabled_rules = ["spotify-cache"]
purge_paths = ["~/personal", "~/work"]
```

A **config path** is `~/…` or an absolute path, with no `*`, `?`, `.` or `..` components.

## Requirements

### Loading
- **CF-01** A missing config file means the defaults: an empty whitelist, no disabled rules, no purge paths, and `dry_run = false`.
- **CF-02** If the file exists but can't be read or parsed, onyx stops before scanning or deleting anything (P-03). The error names the file, and the line where there is one.
- **CF-03** Unknown keys and values of the wrong type are errors.

### Values
- **CF-04** `whitelist` and `purge_paths` entries must be config paths. Anything else (relative paths, globs, `..`, `$VARS`) is an error. `~` resolves from the context passed in, never from the process (as RE-07).
- **CF-05** Each whitelist entry is passed to safe delete, which refuses the path, everything inside it and every directory above it (SD-03).
- **CF-06** Each `disabled_rules` entry must name a loaded rule. An unknown id is an error, because a typo would leave on a rule the user meant to switch off. Disabled rules are never matched.

### Dry run by default
- **CF-07** When `dry_run = true` in the config, or the environment has `ONYX_DRY_RUN=1`, every command runs as a dry run (P-05) unless `--live` is given.
  - `--dry-run` always means a dry run, whatever the config says.
  - `ONYX_DRY_RUN` set to anything but `0` or `1` is an error (P-03).
  - The decision records why a run is a dry run (flag, config or environment). The CLI (004) rejects `--dry-run` with `--live` as a usage error and tells the user when the config or environment made a run a dry run, and how to act (`--live`).

## Test strategy

- Parse tests from strings for every error in CF-02 to CF-04 and CF-06.
- Loading tests use a temp dir as `$XDG_CONFIG_HOME`; an unreadable file is a directory named `config.toml`, which fails even as root.
- CF-07 is a pure function with a table test. Its CLI side is tested in 004 (#3).

## Decisions

- **Dry run by default is a setting** (CF-07), agreed with the maintainer.
- **Fresh installs act normally** (`dry_run = false` without a config). Mutating commands still preview and ask for confirmation (P-06), and need `--yes` when they can't ask.
- **The whitelist takes literal paths only.** An entry already protects everything below it (SD-03).
