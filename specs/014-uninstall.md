# 014: `onyx uninstall`

Status: Implemented
Prefix: UN

## Goal

`onyx uninstall <app>` removes what an app left in your home: its settings, data, state and cache in the XDG folders, its legacy dot-folder, and its menu and autostart entries. You name the app; onyx looks only at the exact paths that name gives, shows the full dry run, and moves what you confirm to the trash through safe delete (001). It covers the home leftovers (#45), removing the program through the system package manager (#46), and flatpak, snap and AppImage apps (#47).

## Non-goals

- Guessing. No fuzzy or partial matches, no searching the disk, no reading `.desktop` files to discover other names. A name that finds nothing removes nothing.
- Removing files outside your home itself. The program goes only through its package manager's own command (P-07), run with `sudo` as `onyx system` does (P-04). onyx never runs as root.
- Picking a package by anything but its exact name, or the exact program name on `$PATH`.
- Package managers' configuration in `/etc` (`apt-get purge`). `remove` is the gentler default.

## Model

1. Start up (004) and check the name (UN-01, UN-03).
2. Build in memory one rule per location in UN-02, all for that one name, with the name as the rule's process (P-08). These rules exist only for this run; they are not built-in rules and `scan` and `clean` never see them.
3. Leave them all out if the app is running (UN-04), then go on exactly as `clean` does: the whitelist (003), measuring with a safe-delete dry run, the preview, one question, safe delete, the report and the operation log (CN-04 to CN-12).
4. Before the preview, find the package that owns the app (UN-08), and show its removal in the same preview (UN-10). On yes, remove the package first through `sudo`, then the leftovers only if that worked (UN-12).

## Package managers

The first one found on the absolute `$PATH` entries (SC-10) that owns the app is used, in this order. Queries and previews run without `sudo`.

| Manager | Found by | Owns `N` when | Preview (no sudo) | Live command |
|---|---|---|---|---|
| pacman (and AUR helpers' packages) | `pacman` | `pacman -Qq N` or `pacman -Qqo <N on $PATH>` names it | `pacman -Rs --print <pkg>` (`-n` and `--print` conflict; `-n` only changes saved config files) | `pacman -Rns <pkg>` |
| apt | `dpkg-query`, `apt-get` | `dpkg-query -W N` shows it installed, or `dpkg-query -S <N on $PATH>` names it | `apt-get -s remove <pkg>` | `apt-get remove <pkg>` |
| dnf | `rpm`, `dnf` | `rpm -q N`, or `rpm -qf <N on $PATH>` names it | `rpm -q <pkg>` | `dnf remove <pkg>` |
| zypper | `rpm`, `zypper` | as dnf | `rpm -q <pkg>` | `zypper remove <pkg>` |
| flatpak (yours) | `flatpak` | `flatpak info --user N` succeeds | `flatpak info --user N` | `flatpak uninstall --user N`, **without** `sudo` |
| flatpak (system) | `flatpak` | `flatpak info --system N` succeeds | `flatpak info --system N` | `flatpak uninstall --system N` |
| snap | `snap` | `snap list N` succeeds | `snap list N` | `snap remove N` |

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
| `uninstall-flatpak-data` | `~/.var/app/N` | user-content (trash) |
| `uninstall-snap-data` | `~/snap/N` | user-content (trash) |
| `uninstall-appimage` | `~/Applications/N.AppImage`, `~/Applications/N.appimage`, `~/.local/bin/N.AppImage`, `~/.local/bin/N.appimage` | user-content (trash) |
| `uninstall-icon` | `$XDG_DATA_HOME/icons/hicolor/*/apps/N.png`, `….svg` | user-content (trash) |

## Requirements

- **UN-01** The name is 1 to 100 characters of ASCII letters, digits, `.`, `_`, `+` and `-`, and doesn't start with `.` or `-`. Anything else, including `/`, `..`, spaces and glob characters, is a usage error (exit 2) that names the rule broken, and nothing is read.
- **UN-02** onyx looks at the locations above and nowhere else, with the name matched exactly and case-sensitively. A directory or file at a location is offered whole. A symlink is skipped (RE-10), never followed.
- **UN-03** Names of folders that many apps share are refused with exit 2: `applications`, `autostart`, `icons`, `fonts`, `mime`, `themes`, `sounds`, `Trash`, `systemd`, `environment.d`, `dbus-1`, `keyrings`, `flatpak`, `config`, `cache`, `local`, `ssh`, `gnupg`, `pki`, and `onyx` (its log is the record of what onyx removed). Safe delete's protected paths (SD-01, SD-02) still apply on top of this.
- **UN-04** If a program with the name is running (P-08), everything is left out and onyx says to quit it first. Nothing is removed and the exit code is 0, as with a busy rule in `clean`.
- **UN-05** Settings, data, state, the dot-folder and the entries go to the trash. The cache is deleted, like every regenerable rule. `--permanent` deletes everything instead, after the same preview and question.
- **UN-06** Everything else is `clean`'s: the full preview first (P-14), one question with no as the default, `--yes` needed without a terminal, `--dry-run` stops after the preview, `--json`, the whitelist, the result and exit codes (CN-11), and the operation log.
- **UN-07** When nothing is found, onyx says so ("Nothing of N in your home") and exits 0. When no package owns the app (UN-08), the preview ends by saying the program itself isn't removed.

### The package (#46)
- **UN-08** The package is the one named exactly `N`, or else the one that owns the program `N` found on the absolute `$PATH` entries, as the table's queries say. No package is guessed from a partial name. If no manager owns it, uninstall goes on with the leftovers only (UN-07).
- **UN-09** Queries and previews run as SY-04 does: without `sudo`, no shell, stdin closed, `$PATH` limited to its absolute entries. A query that fails or can't start means "not owned" by that manager.
- **UN-10** The preview (P-14) shows the package, its manager, everything the preview command printed (for pacman, each package that would go), and the exact live command with the absolute `sudo` path in front, above the leftovers. A dry run stops there and never runs `sudo` (SY-05).
- **UN-11** One question covers both, default no (UN-06). The live command runs as `sudo -- <tool path> <args>` (SY-07) with stdin from the terminal, and without `--noconfirm` or `-y`, so the package manager shows its own list of what it will remove and asks again. `--yes` skips only onyx's question.
- **UN-12** The package goes first. If its command fails or is declined at the manager's own prompt (non-zero exit), the leftovers are kept and the run exits 3, because the app is still installed.
- **UN-13** The removal is in the operation log as a command entry (SY-11), and the result shows its exit status (SY-10). `--json` adds a `package` object (manager, name, preview, command) to the preview and a `package` result.
- **UN-14** A running app (UN-04) leaves the package out too.
- **UN-15** onyx never runs as root (CL-07). Without `sudo` found on an absolute `$PATH` entry, the package is reported but left out, and the leftovers still go ahead.

### Flatpak, snap and AppImage (#47)
- **UN-16** Flatpak and snap are tried after the system managers, in the table's order, by the exact name: a flatpak's application id (`org.mozilla.firefox`), or a snap's name. Everything in UN-08 to UN-14 applies to them.
- **UN-17** A flatpak installed for you is removed with `flatpak uninstall --user` and no `sudo`, so it needs neither root nor `sudo` (P-04). A system flatpak and a snap go through `sudo` as UN-11 says.
- **UN-18** Flatpak asks its own question as the system managers do. `snap remove` doesn't ask, but snapd keeps a snapshot of the snap's data, so onyx's question is the only one there.
- **UN-19** `~/.var/app/N` (a flatpak's data) and `~/snap/N` (a snap's) are leftovers like the others: in the preview, kept if the removal fails (UN-12), and moved to the trash.
- **UN-20** An AppImage is the file `N.AppImage` (or `N.appimage`) in `~/Applications` or `~/.local/bin`, plus its icons named `N.png` or `N.svg` in the hicolor theme. They go to the trash like the other leftovers. A file named after a version (`N-1.2.AppImage`) isn't matched, as that would be a partial match.
- **UN-21** A symlink at any of these paths is skipped (RE-10), as `~/.local/bin` often holds links to programs elsewhere.

## Test strategy

- Smoke tests on a temp home with every location filled for one app, a second app whose name shares a prefix (`foo` and `foo-bar`: only `foo` is touched), a symlink at one location, and the whitelist covering another.
- UN-01, UN-03: each bad and shared name exits 2 and leaves a snapshot of the temp home unchanged.
- UN-04: a copied program named after the app, kept running, leaves everything in place.
- UN-05: the default run puts settings in the temp home's trash and deletes the cache; `--permanent` puts nothing in the trash.
- P-05: `--dry-run` and "no" leave the snapshot unchanged.
- UN-16 to UN-21: fake `flatpak` and `snap` on the temp `$PATH`. A user flatpak's removal is recorded without `sudo`, a system flatpak's and a snap's with it. A temp home with `~/.var/app/N`, `~/snap/N`, `~/Applications/N.AppImage`, a versioned AppImage (kept), an icon, and a symlinked `~/.local/bin/N.AppImage` (kept).
- UN-08 to UN-15: fake `pacman`, `dpkg-query`, `apt-get`, `rpm`, `dnf` and `sudo` scripts on a temp `$PATH` that record their arguments and never touch the system, as in 007's tests. A dry run records no `sudo`. The `sudo` arguments start with `--` and an absolute tool path, with no `--noconfirm` or `-y`. A failing fake removal keeps the leftovers and exits 3. A relative `$PATH` entry is ignored.

## Decisions

- **Exact and case-sensitive.** `code` doesn't match `~/.config/Code`; you type the folder's name. Guessing is how a cleaner deletes the wrong thing.
- **Rules built in memory**, so uninstall reuses `clean`'s whole path instead of a second one.
- **No `~/.local/bin/N`.** That's the program, which #46 and #47 handle.
- **The manager asks again.** Removing a package can take dependencies with it, and only the manager knows its transaction for sure. It asks after onyx's preview, so nothing is removed without two yeses.
- **Versioned AppImages aren't matched.** Exact names only. A user can rename the file or remove it with `onyx analyze`.
- **No `--delete-data`.** Flatpak would delete `~/.var/app/N` itself; onyx moves it to the trash instead, so it can come back.
- **`pacman -Rns`**: also removes dependencies that nothing else needs, and the package's saved config backups. `apt-get remove` and `dnf remove` already handle dependencies their own way.
