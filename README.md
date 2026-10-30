# molt

Reclaim disk space on Linux, safely.

molt finds what is filling your disk (caches, build artifacts, old packages, installers, leftovers from removed apps), shows you exactly what it would remove, and removes only what it can prove is safe to remove. It works on any Linux distribution, with extra rules for pacman, apt, dnf, zypper, flatpak and snap.

> **Status: pre-release.** The foundation and specs are in place; cleaning commands are being built spec-first. Nothing in this version deletes files.

## Planned commands

| Command | What it does |
|---|---|
| `molt clean` | Remove caches, logs and temp files: user, browser, developer tools, package managers |
| `molt purge` | Remove rebuildable project artifacts (`target`, `node_modules`, `build`…) |
| `molt installer` | Find leftover `.deb` / `.rpm` / `.AppImage` / `.iso` files |
| `molt analyze` | Interactive disk explorer |
| `molt status` | Live system health dashboard |
| `molt uninstall` | Remove an app and its leftovers, across package managers |
| `molt optimize` | Safe maintenance: orphan packages, journal vacuum, TRIM, font cache, failed units |
| `molt history` | Show what molt has removed |

Every command that changes anything supports `--dry-run` and `--json`.

## Safety

- Asks for confirmation and shows the plan before changing anything.
- Never follows symlinks, never crosses filesystems, and rechecks every file's identity right before removing it.
- Refuses to touch `$HOME`, `~/.ssh`, `~/.gnupg`, keyrings, and system directories.
- Runs without root; privileged steps are explicit and opt-in.
- Logs every change to `~/.local/state/molt/operations.log`.

See [`specs/000-principles.md`](specs/000-principles.md) for the full set of rules.

## Install

Download the latest release from the [Releases page](https://github.com/sumdahl/molt/releases), then:

```sh
tar xzf molt-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz
sha256sum -c molt-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz.sha256
install -Dm755 molt-vX.Y.Z-x86_64-unknown-linux-musl/molt ~/.local/bin/molt
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
