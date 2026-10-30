# molt

Reclaim disk space on Linux, safely.

molt finds what is filling your disk (caches, build artifacts, old packages, installers, leftovers from removed apps), shows you exactly what it would remove, and removes only what it can prove is safe to remove. It works on any Linux distribution. Distro-specific steps turn on only where their tools are installed; pacman is the first, with apt, dnf, zypper, Flatpak and Snap planned.

> **Status: v0.1.0, early.** `molt scan`, `molt clean`, `molt purge`, `molt installer`, `molt uninstall`, `molt system`, `molt analyze` and `molt status` work. Other commands are being built spec-first.

## Commands

`molt scan` (or just `molt`) shows how much space molt could free, by category, without changing anything:

```text
$ molt scan
Developer tools                               21.3 GiB
  npm download cache                          21.0 GiB  via `npm cache clean --force`
System (your files)                            1.2 GiB
  Thumbnail cache                              1.2 GiB

Total: 22.5 GiB can be freed with `molt clean`.
```

`molt scan --json` prints every path, its size, and anything skipped and why.

`molt clean` frees that space. It always shows the full preview first, asks once (default no), and then:

- runs a tool's own cleanup command where it has one (`npm cache clean --force`), and never also deletes that tool's files,
- leaves alone the cache of any program that is running,
- skips anything that changed since the preview,
- reports what it freed, and logs every change to `~/.local/state/molt/operations.log`.

```sh
molt clean --dry-run               # the preview only
molt clean --only npm-cache        # just one rule
molt clean --skip maven-repository # everything but one
molt clean --yes                   # no question (the preview is still printed)
molt clean --trash                 # move to the trash instead of deleting
```

Folders you own but can't read (makepkg leaves these in AUR caches) are kept unless you agree to open them: molt asks, or pass `--fix-permissions`. `--yes` alone never opens them.

`molt purge` removes the build output of your own projects (`target`, `node_modules`, `.venv`, Gradle `build`, `.next`, Zig caches) under the folders you list in `purge_paths`. A build folder counts only when its project file sits next to it (`Cargo.toml` beside `target`). Projects changed in the last 7 days and folders holding Git-tracked files are left out. It shows the same preview and asks the same question as `clean`.

```sh
molt purge --dry-run               # what would go
molt purge --only cargo node       # just these kinds
molt purge --trash                 # to the trash instead
```

`molt installer` moves old installers (`.deb`, `.rpm`, `.pkg.tar.zst`, `.AppImage`, `.iso`, `.run`) from `~/Downloads` and `~/Desktop` to the trash, so you can get them back. Files from the last 14 days, folders and symlinks are left alone. `clean` never touches them.

```sh
molt installer --dry-run           # what would go
molt installer --permanent         # delete instead of trashing
```

`molt uninstall <app>` removes what an app left in your home: its folder in `~/.config`, `~/.local/share`, `~/.local/state` and `~/.cache`, its `~/.<app>` dot-folder, and its menu and autostart entries. The name must match the folder exactly (`Code`, not `code`), and names of shared folders like `applications` or `Trash` are refused. Nothing is touched while the app is running. The cache is deleted; everything else goes to the trash. The program itself is left to your package manager for now.

```sh
molt uninstall chromium --dry-run  # what it left, and where
molt uninstall Code --permanent    # delete instead of trashing
```

`molt analyze [PATH]` shows where the space in a folder goes, largest first, without changing anything. On a terminal it opens a full-screen browser (`j`/`k` or arrows to move, `l`/Enter to open, `h` to go up, `s` to sort, `r` to measure again, `Space` to mark, `d` to move marked items to the trash, `?` for help, `q` to quit); piped, or with `--list`, it prints a listing: on-disk sizes, hard links counted once, symlinks and other filesystems never entered. A `+` marks a size that's a lower bound because something inside couldn't be read.

```sh
molt analyze ~                     # browse your home
molt analyze ~ --list --depth 2    # print two levels, 20 largest per level
molt analyze --all --json          # everything, machine-readable
```

`molt status` shows the machine's health in a fifth of a second, read from `/proc` and `/sys` without root: CPUs and how busy they are, load, uptime, memory and swap, each real disk (pointing at `molt analyze` when one is 90% full), processes (naming any zombies), disk and network speeds, temperatures and fans, and the battery. It ends with a health score out of 100 that lists every point it took off and why; the formula is in `specs/013-status.md`. `--json` for scripts. `--watch` shows it full screen and refreshes it every 2 seconds (`--interval` 1 to 60), with rates measured over the whole period; `p` pauses, `r` refreshes now, `q` quits.

```sh
molt status                        # one snapshot
molt status --watch --interval 5   # live, every 5 seconds
```

`molt system` frees space outside your home through each tool's own command, run with `sudo`: the pacman cache (`paccache -rk2` and `-ruk0`), the systemd journal (`journalctl --vacuum-size=500M`) and coredumps past the system's own age limit (`systemd-tmpfiles --clean`). molt itself never runs as root. The preview uses each tool's read-only mode without `sudo`, so `molt system --dry-run` never asks for a password. A task is shown only when its tools are installed.

```sh
molt system --dry-run              # sizes and the exact sudo commands
molt system --only journal         # just one task
```

### Planned

| Command | What it does |
|---|---|
| `molt uninstall` (program) | Remove the program too: pacman, apt, dnf, zypper, flatpak, snap, AppImage |
| `molt optimize` | Safe maintenance: orphan packages, TRIM, font cache, failed units |
| `molt history` | Show what molt has removed |

## Options and configuration

These work with every command:

| Flag | What it does |
|---|---|
| `--dry-run` | Show what would happen without changing anything |
| `--live` | Act, even when the config or `MOLT_DRY_RUN` asks for a dry run |
| `--yes` | Don't ask for confirmation |
| `--json` | Machine-readable output, errors included |

Settings live in `~/.config/molt/config.toml`. Every key is optional:

```toml
dry_run = true                        # preview only, unless --live
whitelist = ["~/work/keep-this", "/data/vms"]   # never touched, nor anything inside
disabled_rules = ["thumbnails"]       # built-in rules to switch off
purge_paths = ["~/projects"]          # where `molt purge` looks
```

If the config or a rule file in `~/.config/molt/rules/` can't be read exactly, molt stops without doing anything.

| Variable | Effect |
|---|---|
| `MOLT_DRY_RUN=1` | Like `dry_run = true` |
| `MOLT_NO_OPLOG=1` | Don't write the operation log |

molt refuses to run as root.

Exit codes: `0` success, `1` fatal error with nothing changed, `2` bad usage, `3` partial success.

## Safety

- Asks for confirmation and shows the plan before changing anything.
- Never follows symlinks, never crosses filesystems, and rechecks every file's identity right before removing it.
- Refuses to touch `$HOME`, `~/.ssh`, `~/.gnupg`, keyrings, and system directories.
- Runs without root; privileged steps are explicit and opt-in.
- Logs every change to `~/.local/state/molt/operations.log`.

See [`specs/000-principles.md`](specs/000-principles.md) for the full set of rules.

## Install

Download v0.1.0, the latest release, from the [Releases page](https://github.com/sumdahl/molt/releases), then:

```sh
tar xzf molt-v0.1.0-x86_64-unknown-linux-musl.tar.gz
sha256sum -c molt-v0.1.0-x86_64-unknown-linux-musl.tar.gz.sha256
install -Dm755 molt-v0.1.0-x86_64-unknown-linux-musl/molt ~/.local/bin/molt
```

From source:

```sh
cargo install --git https://github.com/sumdahl/molt
```

## Development

molt is developed spec-first, with AI agents as first-class contributors. Start with [`AGENTS.md`](AGENTS.md), then [`specs/`](specs/).

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

## Acknowledgements

Inspired by [Mole](https://github.com/tw93/mole) for macOS. molt is an independent implementation, written from scratch in Rust.

## License

[MIT](LICENSE)
