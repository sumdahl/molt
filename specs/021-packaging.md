# 021: Packages: AUR, `.deb` and crates.io

Status: Accepted (#62, #63, #64)
Prefix: PK

## Goal

molt can be installed by the package manager people already use: `molt-bin` from the AUR, a `.deb` attached to each release, and `cargo install molt-cli`. Every package ships the same signed release binary (AUR, `.deb`) or the same tagged source (crates.io), with the README, licence and shell completions, and its package manager owns the files from then on.

## Non-goals

- Publishing. Pushing to the AUR and running `cargo publish` are the maintainer's steps; molt's repository only holds what they publish and the checks that it works. The AUR isn't open to new accounts yet, so until then the filled `PKGBUILD` on each release is built by hand (`makepkg -si`).
- An apt repository, a PPA, a `.rpm`, Flatpak, Snap or Homebrew.
- A `molt` (built from source) AUR package; `molt-bin` only.
- aarch64 packages. Only the x86_64 musl build exists (SE-01).

## Requirements

### Shared
- **PK-01** Each package installs `molt` to the system's binary folder, the licence, the README, and the completions molt prints itself (`molt completions <shell>`, CO-01) at the shells' system paths: `/usr/share/bash-completion/completions/molt`, `/usr/share/zsh/site-functions/_molt` and `/usr/share/fish/vendor_completions.d/molt.fish`. The completions are generated from the packaged binary when the package is built, so they can't drift from it. (crates.io can't install files beyond the binary; README says how to add the completions.)
- **PK-02** A packaged molt is the package manager's (SE-04): `molt update` and `molt remove` refuse it and name the command to use, `pacman`/your AUR helper for `/usr/bin/molt` installed by pacman, `apt` for one installed by dpkg, and `cargo install molt-cli` / `cargo uninstall molt-cli` for one in `$CARGO_HOME/bin` (default `~/.cargo/bin`). That last one is in your home and yours, so SE-04 changes: a binary in `$CARGO_HOME/bin` is cargo's, not molt's to replace.

### AUR: `molt-bin` (#62)
- **PK-03** `packaging/aur/PKGBUILD` builds `molt-bin` from the release tarball on GitHub: `provides=(molt)`, `conflicts=(molt)`, `arch=(x86_64)`, `license=(MIT)`, the release's tarball as its only source, and its sha256. It never builds from source and never downloads during `package()`.
- **PK-04** The release workflow fills the PKGBUILD's `pkgver` and `sha256sums` from the tarball it just built, builds it with `makepkg` in an `archlinux` container (as a normal user, with the tarball already in place, so nothing is downloaded), installs the result with `pacman -U`, and checks that `molt --version` and the three completion files are there. It then attaches the filled `PKGBUILD` and `.SRCINFO` to the GitHub Release, ready for the maintainer to push to the AUR.

### `.deb` (#63)
- **PK-05** The release workflow builds `molt_X.Y.Z_amd64.deb` with `dpkg-deb --root-owner-group` from the musl binary: `/usr/bin/molt`, `/usr/share/doc/molt/copyright` (the licence) and `README.md`, and the completions. Its `control` names the package `molt`, `Architecture: amd64`, no `Depends` (the binary is static), and the maintainer and homepage.
- **PK-06** The workflow installs the `.deb` with `apt-get install ./molt_….deb` in a `debian:stable` container, checks `molt --version` and the completion files, removes it with `apt-get remove`, and checks `/usr/bin/molt` is gone. It attaches the `.deb` and its `.sha256` to the release. The maintainer signs that `.sha256` too, with the same command as the tarball's (AGENTS.md step 7).

### crates.io: `molt-cli` (#64)
- **PK-07** `Cargo.toml` names the package `molt-cli`, with `[lib] name = "molt"` and `[[bin]] name = "molt"`, so the command and `use molt::…` stay as they are. It has `description`, `license`, `repository`, `readme`, `keywords` and `categories`, `publish = true`, and an `include` list of exactly what the build needs: `src/`, `Cargo.toml`, `Cargo.lock`, `README.md`, `LICENSE`, `CHANGELOG.md` and `.github/allowed_signers` (built into molt, SE-09's built-in signers).
- **PK-08** CI runs `cargo publish --dry-run --locked`, which builds the packaged crate on its own, so a file missing from `include` fails the build. The release workflow runs it too; the maintainer runs the real `cargo publish` after the release.

## Test strategy

- PK-01, PK-04, PK-06: the release workflow's container steps install each package and check the files and `molt --version`; a dry run of those steps runs in CI on every PR against a locally built musl tarball, so they're tested before a tag needs them.
- PK-02: smoke tests run a copy of molt from `$CARGO_HOME/bin` in the temp home (refused, naming `cargo install molt-cli`), and unit tests map `/usr/bin/molt` with a fake `pacman -Qo` or `dpkg -S` to the right message.
- PK-03: `bash -n` and a test that sources the PKGBUILD in bash and checks its fields; `namcap` where installed.
- PK-07, PK-08: `cargo publish --dry-run --locked` in CI.

## Decisions

- **Maintainer line:** `Sumiran Dahal <50270034+sumdahl@users.noreply.github.com>`, the GitHub address the release commits already use, so no personal email is published.
- **No AUR account yet** (maintainer, 2026-10-30): the AUR isn't letting new users register. The PKGBUILD and `.SRCINFO` are still built, tested and attached to each release; pushing them to the AUR waits for an account.
- **A cargo-installed molt is cargo's** (maintainer, 2026-10-30): `molt update` and `molt remove` refuse a binary in `$CARGO_HOME/bin` and name `cargo install molt-cli` / `cargo uninstall molt-cli` (PK-02).
- **Names checked (2026-10-30):** `molt-cli` is free on crates.io (`molt` is taken), and neither `molt-bin` nor `molt` exists on the AUR.
- **The PKGBUILD lives in this repo and the workflow fills it.** The release's own sums go in, so the maintainer copies two files to the AUR instead of editing them by hand.
- **Packages from the signed release binary**, not rebuilt, so every channel ships the exact bytes the maintainer signed.
