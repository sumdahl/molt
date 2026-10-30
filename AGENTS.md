# AGENTS.md

Guide for AI agents (and humans) working on onyx, a Linux disk cleaner written in Rust. Read this before changing anything.

## The one rule

onyx deletes files. A bug here destroys someone's data. When in doubt, refuse, skip, and report (P-02).

## Workflow: one issue at a time

All work is tracked as GitHub issues, grouped into milestones by phase.

1. **Pick** the lowest-numbered open issue in the earliest open milestone, unless the maintainer names another. Work on one issue at a time.
2. **Break down** an issue that is too big for one PR, or an `epic` whose phase has started, into new issues first. Link them from the parent.
3. **Branch** from `main` as `<issue>-<slug>`, e.g. `2-config-whitelist`.
4. **Commit** in small Conventional Commits that reference the issue (`feat: load config (#2)`).
5. **Open a PR** whose body says `Closes #N` and summarises what changed and how it was tested. CI must be green.
6. **Merge** once the issue is implemented and CI is green: squash-merge, then delete the branch. The maintainer has given standing approval for this. Spec acceptance and releases still need the maintainer.
7. **Update the issue** when plans change. Questions for the maintainer go in the issue, as well as in chat.

## Workflow: spec-first

1. **No behaviour without a spec.** Find the spec in `specs/`. If none covers the change, write or extend one (see `specs/README.md`) and get it accepted before writing code.
2. **Tests before code.** Write failing tests that name the requirement ID (`fn sd_05_…`). Then implement until they pass.
3. **Code cites the spec** where the reason isn't obvious: `// SD-09: unlinkat relative to the dir fd, never by path`.
4. **Keep the spec true.** If the implementation shows a requirement is wrong, change the spec in the same PR. Never let the code and the spec quietly disagree.
5. **Mark progress.** When every requirement in a spec has a passing test, set its status to `Implemented` in the spec and in the `specs/README.md` index.

## Safety invariants (from specs/000)

- Delete only through the safe-delete module. Clippy forbids `std::fs::remove_*`, `std::fs::rename`, `rustix::fs::unlinkat` and `rustix::fs::renameat*` everywhere else. Never add `#[allow(clippy::disallowed_methods)]` outside that module.
- Never follow symlinks during deletion. Never cross filesystems.
- Fail closed on unreadable config.
- Never require root by default.

## Code rules

These are rules, not suggestions. Code that breaks them doesn't merge.

- **Safe Rust only.** `#![forbid(unsafe_code)]` is set at the crate root. Reach the kernel through `rustix`, never through raw `libc` calls.
- **Idiomatic Rust.** Use the type system to rule out mistakes (enums over flags and strings, newtypes over bare integers). Return errors with `Result` and `?`. Never `unwrap()` or `expect()` outside tests. Implement standard traits (`Debug`, `Display`, `Error`, `From`) instead of inventing your own.
- **Clean code.** Small functions that do one thing, names that say what a thing is, no dead code, no speculative abstractions. A comment explains *why*, citing the spec ID; the code says *what*.

## Tests

- **Never touch the real home directory.** Every test that touches the filesystem uses a temporary directory as `$HOME` with `XDG_*` pointed inside it.
- In test builds, `safe_delete::execute` panics if a live plan names anything outside the system temp directory. Don't weaken or bypass this tripwire.
- Use real filesystem operations; don't mock the filesystem.
- `tests/smoke.rs` must always pass: the binary starts, `--version` matches `Cargo.toml`, and bad flags exit with code 2.
- Agents must never run onyx against the real system except with `--dry-run`. Even then, read the dry run's output before anything else (P-14).

## Commands

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

All three must pass before every commit. CI runs the same three.

## Every PR

- Update `CHANGELOG.md` under `## [Unreleased]` for any user-visible change.
- Update `README.md` when commands, flags, or install steps change.
- Use Conventional Commits (`feat:`, `fix:`, `docs:`, `spec:`, `test:`, `chore:`).

## Releasing

Releases are cut from `main` by pushing a tag. `.github/workflows/release.yml` does the rest.

1. Pick the version (SemVer; while on `0.x`, a breaking change bumps the minor).
2. Set `version` in `Cargo.toml` and run `cargo check` so `Cargo.lock` is updated.
3. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD` and add a fresh empty `## [Unreleased]` above it.
4. Update `README.md`, including the version shown in the install snippet.
5. Commit `chore: release vX.Y.Z`, then sign the tag with a maintainer's SSH key listed in `.github/allowed_signers`: `git -c gpg.format=ssh -c user.signingkey=~/.ssh/id_personal.pub tag -s vX.Y.Z -m "onyx vX.Y.Z"`, and `git push origin vX.Y.Z`. Releases are always signed; never use `git tag -a`.
6. The workflow then:
   - refuses a tag that isn't signed by a key in `.github/allowed_signers`,
   - checks that the tag matches `Cargo.toml` and that `CHANGELOG.md` has a section for the version,
   - runs fmt, clippy and the tests,
   - builds a static musl binary and smoke-tests it,
   - publishes a GitHub Release with that changelog section as the notes, plus the tarball and its sha256.
7. The maintainer signs the checksum, so `onyx update` will install the release (specs/020-self.md):
   `gh release download vX.Y.Z -p '*.sha256'`, then
   `ssh-keygen -Y sign -f ~/.ssh/id_personal -n onyx-release onyx-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz.sha256`, then
   `gh release upload vX.Y.Z onyx-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz.sha256.sig`.

Agents prepare the release commit. A human confirms before the tag is pushed. The signing key has a passphrase, so the maintainer signs the tag (or loads the key into `ssh-agent` first).
