# 014: `molt uninstall`

Status: Accepted (#45)
Prefix: UN

## Goal

`molt uninstall <app>` removes what an app left in your home: its settings, data, state and cache in the XDG folders, its legacy dot-folder, and its menu and autostart entries. You name the app; molt looks only at the exact paths that name gives, shows the full dry run, and moves what you confirm to the trash through safe delete (001). This spec covers the home leftovers (#45). Removing the program itself through the package manager (#46), and flatpak, snap and AppImage (#47), add requirements here later.

## Non-goals

- Guessing. No fuzzy or partial matches, no searching the disk, no reading `.desktop` files to discover other names. A name that finds nothing removes nothing.
- Removing the program. Until #46, molt says the program itself is left for your package manager.
- Anything outside your home. No root.

## Model

1. Start up (004) and check the name (UN-01, UN-03).
2. Build in memory one rule per location in UN-02, all for that one name, with the name as the rule's process (P-08). These rules exist only for this run; they are not built-in rules and `scan` and `clean` never see them.
3. Leave them all out if the app is running (UN-04), then go on exactly as `clean` does: the whitelist (003), measuring with a safe-delete dry run, the preview, one question, safe delete, the report and the operation log (CN-04 to CN-12).

## Locations

For a name `N`:

| Rule | Path | Kind (default mode) |
|---|---|---|
| `uninstall-config` | `$XDG_CONFIG_HOME/N` | user-content (trash) |
| `uninstall-data` | `$XDG_DATA_HOME/N` | user-content (trash) |
| `uninstall-state` | `$XDG_STATE_HOME/N` | user-content (trash) |
| `uninstall-cache` | `$XDG_CACHE_HOME/N` | regenerable (delete) |
| `uninstall-dotfolder` | `~/.N` | user-content (trash) |
| `uninstall-menu-entry` | `$XDG_DATA_HOME/applications/N.desktop` | user-content (trash) |
| `uninstall-autostart` | `$XDG_CONFIG_HOME/autostart/N.desktop` | user-content (trash) |

## Requirements

- **UN-01** The name is 1 to 100 characters of ASCII letters, digits, `.`, `_`, `+` and `-`, and doesn't start with `.` or `-`. Anything else, including `/`, `..`, spaces and glob characters, is a usage error (exit 2) that names the rule broken, and nothing is read.
- **UN-02** molt looks at the locations above and nowhere else, with the name matched exactly and case-sensitively. A directory or file at a location is offered whole. A symlink is skipped (RE-10), never followed.
- **UN-03** Names of folders that many apps share are refused with exit 2: `applications`, `autostart`, `icons`, `fonts`, `mime`, `themes`, `sounds`, `Trash`, `systemd`, `environment.d`, `dbus-1`, `keyrings`, `flatpak`, `config`, `cache`, `local`, `ssh`, `gnupg`, `pki`, and `molt` (its log is the record of what molt removed). Safe delete's protected paths (SD-01, SD-02) still apply on top of this.
- **UN-04** If a program with the name is running (P-08), everything is left out and molt says to quit it first. Nothing is removed and the exit code is 0, as with a busy rule in `clean`.
- **UN-05** Settings, data, state, the dot-folder and the entries go to the trash. The cache is deleted, like every regenerable rule. `--permanent` deletes everything instead, after the same preview and question.
- **UN-06** Everything else is `clean`'s: the full preview first (P-14), one question with no as the default, `--yes` needed without a terminal, `--dry-run` stops after the preview, `--json`, the whitelist, the result and exit codes (CN-11), and the operation log.
- **UN-07** When nothing is found, molt says so ("Nothing of N in your home") and exits 0. The preview always ends by saying that the program itself isn't removed.

## Test strategy

- Smoke tests on a temp home with every location filled for one app, a second app whose name shares a prefix (`foo` and `foo-bar`: only `foo` is touched), a symlink at one location, and the whitelist covering another.
- UN-01, UN-03: each bad and shared name exits 2 and leaves a snapshot of the temp home unchanged.
- UN-04: a copied program named after the app, kept running, leaves everything in place.
- UN-05: the default run puts settings in the temp home's trash and deletes the cache; `--permanent` puts nothing in the trash.
- P-05: `--dry-run` and "no" leave the snapshot unchanged.

## Decisions

- **Exact and case-sensitive.** `code` doesn't match `~/.config/Code`; you type the folder's name. Guessing is how a cleaner deletes the wrong thing.
- **Rules built in memory**, so uninstall reuses `clean`'s whole path instead of a second one.
- **No `~/.local/bin/N`.** That's the program, which #46 and #47 handle.
