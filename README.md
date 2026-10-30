# onyx

Reclaim disk space on Linux, safely.

onyx finds what is filling your disk (caches, build artifacts, old packages, installers, leftovers from removed apps), shows you exactly what it would remove, and removes only what it can prove is safe to remove. It works on any Linux distribution. Distro-specific steps turn on only where their tools are installed; pacman is the first, with apt, dnf, zypper, Flatpak and Snap planned.

> **Status: v0.2.0, early.** `onyx scan`, `onyx clean`, `onyx purge`, `onyx installer`, `onyx uninstall`, `onyx system`, `onyx optimize`, `onyx history`, `onyx analyze`, `onyx status`, `onyx update` and `onyx remove` work. Other commands are being built spec-first.

## Commands

`onyx scan` (or just `onyx`) shows how much space onyx could free, by category, without changing anything:

```text
$ onyx scan
Developer tools                               21.3 GiB
  npm download cache                          21.0 GiB  via `npm cache clean --force`
System (your files)                            1.2 GiB
  Thumbnail cache                              1.2 GiB

Total: 22.5 GiB can be freed with `onyx clean`.
```

`onyx scan --json` prints every path, its size, and anything skipped and why.

`onyx clean` frees that space. It always shows the full preview first, asks once (default no), and then:

- runs a tool's own cleanup command where it has one (`npm cache clean --force`), and never also deletes that tool's files,
- leaves alone the cache of any program that is running,
- skips anything that changed since the preview,
- reports what it freed, and logs every change to `~/.local/state/onyx/operations.log`.

```sh
onyx clean --dry-run               # the preview only
onyx clean --only npm-cache        # just one rule
onyx clean --skip maven-repository # everything but one
onyx clean --yes                   # no question (the preview is still printed)
onyx clean --trash                 # move to the trash instead of deleting
```

Folders you own but can't read (makepkg leaves these in AUR caches) are kept unless you agree to open them: onyx asks, or pass `--fix-permissions`. `--yes` alone never opens them.

`onyx purge` removes the build output of your own projects (`target`, `node_modules`, `.venv`, Gradle `build`, `.next`, Zig caches) under the folders you list in `purge_paths`. A build folder counts only when its project file sits next to it (`Cargo.toml` beside `target`). Projects changed in the last 7 days and folders holding Git-tracked files are left out. It shows the same preview and asks the same question as `clean`.

```sh
onyx purge --dry-run               # what would go
onyx purge --only cargo node       # just these kinds
onyx purge --trash                 # to the trash instead
```

`onyx installer` moves old installers (`.deb`, `.rpm`, `.pkg.tar.zst`, `.AppImage`, `.iso`, `.run`) from `~/Downloads` and `~/Desktop` to the trash, so you can get them back. Files from the last 14 days, folders and symlinks are left alone. `clean` never touches them.

```sh
onyx installer --dry-run           # what would go
onyx installer --permanent         # delete instead of trashing
```

`onyx uninstall <app>` removes what an app left in your home: its folder in `~/.config`, `~/.local/share`, `~/.local/state` and `~/.cache`, its `~/.<app>` dot-folder, its menu and autostart entries, a flatpak's `~/.var/app/<id>`, a snap's `~/snap/<name>`, and an `<app>.AppImage` in `~/Applications` or `~/.local/bin` with its icons. The name must match the folder exactly (`Code`, not `code`), and names of shared folders like `applications` or `Trash` are refused. Nothing is touched while the app is running. The cache is deleted; everything else goes to the trash.

If pacman, apt, dnf, zypper, flatpak (by app id) or snap has a package named exactly `<app>`, or one that owns the `<app>` program on `$PATH`, the preview shows it with everything its manager would remove, and the exact `sudo -- pacman -Rns <pkg>` (or `apt-get remove`, `dnf remove`, `zypper remove`) it would run. After your yes the package goes first, and the manager still asks its own question. If that fails or you say no, the leftovers are kept. A flatpak installed for you is removed without `sudo`. A dry run never runs `sudo`.

```sh
onyx uninstall chromium --dry-run  # what it left, and where
onyx uninstall Code --permanent    # delete instead of trashing
```

`onyx analyze [PATH]` shows where the space in a folder goes, largest first, without changing anything. On a terminal it opens a full-screen browser (`j`/`k` or arrows to move, `l`/Enter to open, `h` to go up, `s` to sort, `r` to measure again, `Space` to mark, `d` to move marked items to the trash, `?` for help, `q` to quit); piped, or with `--list`, it prints a listing: on-disk sizes, hard links counted once, symlinks and other filesystems never entered. A `+` marks a size that's a lower bound because something inside couldn't be read.

```sh
onyx analyze ~                     # browse your home
onyx analyze ~ --list --depth 2    # print two levels, 20 largest per level
onyx analyze --all --json          # everything, machine-readable
```

`onyx status` shows the machine's health in a fifth of a second, read from `/proc` and `/sys` without root: CPUs and how busy they are, load, uptime, memory and swap, each real disk (pointing at `onyx analyze` when one is 90% full), processes (naming any zombies), disk and network speeds, temperatures and fans, and the battery. It ends with a health score out of 100 that lists every point it took off and why; the formula is in `specs/013-status.md`. `--json` for scripts. `--watch` shows it full screen and refreshes it every 2 seconds (`--interval` 1 to 60), with rates measured over the whole period; `p` pauses, `r` refreshes now, `q` quits.

```sh
onyx status                        # one snapshot
onyx status --watch --interval 5   # live, every 5 seconds
```

`onyx system` frees space outside your home through each tool's own command, run with `sudo`: the pacman cache (`paccache -rk2` and `-ruk0`), the systemd journal (`journalctl --vacuum-size=500M`) and coredumps past the system's own age limit (`systemd-tmpfiles --clean`). onyx itself never runs as root. The preview uses each tool's read-only mode without `sudo`, so `onyx system --dry-run` never asks for a password. A task is shown only when its tools are installed.

```sh
onyx system --dry-run              # sizes and the exact sudo commands
onyx system --only journal         # just one task
```

`onyx optimize` removes packages nothing needs: pacman's orphans (`pacman -Qdtq`), `apt-get autoremove`, dnf's and zypper's unneeded packages. The preview lists every package, found without root, and the exact `sudo` command, which names exactly those packages. The package manager still shows its own list and asks. Then upkeep: TRIM on your SSDs (`sudo -- fstrim --all`), your out-of-date font caches (`fc-cache`, no `sudo`), and the DNS cache (`sudo -- resolvectl flush-caches`, only when systemd-resolved runs). Last, read-only checks: failed systemd units (yours and the system's) and each disk's SMART health (`smartctl -H`, never through `sudo`; onyx names the command when a disk needs root). A dry run never runs `sudo`.

```sh
onyx optimize --dry-run            # what nothing needs any more
onyx optimize --only pacman-orphans
```

`onyx history` shows what onyx has done, newest first: each item it trashed or deleted, and each command it ran, with sizes, from its operation log. It only reads. `--limit N` (default 50), `--all`, `--since YYYY-MM-DD`, `--json`.

```sh
onyx history --since 2026-10-01    # this month's removals
```

### Shell completions

`onyx completions <shell>` prints a completion script for commands, flags and task ids. Put it where your shell looks:

```sh
onyx completions bash > ~/.local/share/bash-completion/completions/onyx
onyx completions zsh > ~/.zfunc/_onyx     # with fpath+=(~/.zfunc) before compinit
onyx completions fish > ~/.config/fish/completions/onyx.fish
```

### The menu

Run `onyx` on its own in a terminal for a menu of every command, under the banner. Pick one with the arrows or `j`/`k` and Enter: it runs exactly as if you'd typed it, dry run and question included, then comes back. Flags given to `onyx` (`--dry-run`, `--yes`) carry over. Piped, a bare `onyx` is still `onyx scan`.

### Update and remove

`onyx update` installs the newest release over the onyx you're running, if that copy is yours and under your home (a package manager's onyx is left to it). It downloads with `curl` into a fresh temp folder, then checks, in order: the checksum is signed by a maintainer (`ssh-keygen -Y verify`, against keys built into onyx), `sha256sum`, the archive holds only plain files in one folder, and the new binary reports the new version. Any failure stops with nothing replaced. The swap is atomic, and the old binary goes to the trash.

`onyx remove` moves onyx's binary, config, cache, state and completion files to the trash (`--permanent` deletes them). The operation log stays unless you add `--with-log`. Both show the full preview first and ask.

## Options and configuration

These work with every command:

| Flag | What it does |
|---|---|
| `--dry-run` | Show what would happen without changing anything |
| `--live` | Act, even when the config or `ONYX_DRY_RUN` asks for a dry run |
| `--yes` | Don't ask for confirmation |
| `--json` | Machine-readable output, errors included |

Settings live in `~/.config/onyx/config.toml`. Every key is optional:

```toml
dry_run = true                        # preview only, unless --live
whitelist = ["~/work/keep-this", "/data/vms"]   # never touched, nor anything inside
disabled_rules = ["thumbnails"]       # built-in rules to switch off
purge_paths = ["~/projects"]          # where `onyx purge` looks
confirm = "fingerprint"               # or "type" (the default); see below
```

With `confirm = "fingerprint"`, a live run's question can be answered by touching the fingerprint sensor (through fprintd's `fprintd-verify`) instead of typing `y`. The full preview still comes first. If there's no reader, no enrolled finger or no match, onyx asks you to type the answer instead, default no. It's a convenience, not a lock: typing `y` always works. Ctrl-C stops with nothing changed. Each yes is logged with how it was given (typed, fingerprint, or `--yes`).

If the config or a rule file in `~/.config/onyx/rules/` can't be read exactly, onyx stops without doing anything.

| Variable | Effect |
|---|---|
| `ONYX_DRY_RUN=1` | Like `dry_run = true` |
| `ONYX_NO_OPLOG=1` | Don't write the operation log |

onyx refuses to run as root.

Exit codes: `0` success, `1` fatal error with nothing changed, `2` bad usage, `3` partial success.

## Safety

- Asks for confirmation and shows the plan before changing anything.
- Never follows symlinks, never crosses filesystems, and rechecks every file's identity right before removing it.
- Refuses to touch `$HOME`, `~/.ssh`, `~/.gnupg`, keyrings, and system directories.
- Runs without root; privileged steps are explicit and opt-in.
- Logs every change to `~/.local/state/onyx/operations.log`.

See [`specs/000-principles.md`](specs/000-principles.md) for the full set of rules.

## Install

Download v0.2.0, the latest release, from the [Releases page](https://github.com/sumdahl/onyx/releases), then:

```sh
f=onyx-v0.2.0-x86_64-unknown-linux-musl.tar.gz
curl -fsSLO https://raw.githubusercontent.com/sumdahl/onyx/main/.github/allowed_signers
ssh-keygen -Y verify -f allowed_signers -I sumdahl -n onyx-release -s $f.sha256.sig < $f.sha256
sha256sum -c $f.sha256
tar xzf $f
install -Dm755 onyx-v0.2.0-x86_64-unknown-linux-musl/onyx ~/.local/bin/onyx
```

Later updates: `onyx update`.

On Arch, each release from v0.3.0 on carries a ready `PKGBUILD` for `onyx-bin` (it's not on the AUR yet). Download it, then:

```sh
mkdir onyx-bin && mv onyx-bin.PKGBUILD onyx-bin/PKGBUILD && cd onyx-bin
makepkg -si
```

pacman then owns onyx, so update and remove it with pacman; `onyx update` says so. The same goes for an onyx from `cargo install`, which cargo updates.

On Debian or Ubuntu, each release from v0.3.0 on has `onyx_X.Y.Z_amd64.deb`, with a signed `.sha256` like the tarball's:

```sh
sudo apt install ./onyx_0.3.0_amd64.deb
```

From crates.io (the crate is `onyx-cli`, because `onyx` was taken; the command is `onyx`), once it's published:

```sh
cargo install onyx-cli
```

cargo then owns that copy: update it with `cargo install onyx-cli` and remove it with `cargo uninstall onyx-cli`. Add the completions yourself (see Shell completions).

From source:

```sh
cargo install --git https://github.com/sumdahl/onyx
```

## Development

onyx is developed spec-first, with AI agents as first-class contributors. Start with [`AGENTS.md`](AGENTS.md), then [`specs/`](specs/).

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

## Acknowledgements

Inspired by [Mole](https://github.com/tw93/mole) for macOS. onyx is an independent implementation, written from scratch in Rust.

## License

[MIT](LICENSE)
