# 020: `onyx update` and `onyx remove`

Status: Implemented (#61)
Prefix: SE

## Goal

`onyx update` replaces onyx with the newest release, but only one the maintainer signed. `onyx remove` takes onyx off the machine: its binary, config and state. Both show a full dry run first and ask (P-14), like every other command.

## Non-goals

- Updating or removing an onyx that a package manager installed (AUR, `.deb`, `/usr/bin`). That package manager owns it; onyx says which command to use.
- Downgrades, channels, or background update checks. onyx never contacts the network unless `onyx update` is run.
- HTTP, TLS or crypto in onyx. `curl` downloads, `sha256sum` checks and `ssh-keygen -Y verify` verifies, as owning tools (P-07). No new dependencies.

## Trust

A release's tarball and its `.sha256` are built by CI from a signed tag. A checksum made by CI proves the download wasn't corrupted, not that the release wasn't tampered with. So releases gain one file:

- `onyx-vX.Y.Z-<target>.tar.gz.sha256.sig`: the `.sha256` signed by the maintainer with `ssh-keygen -Y sign -n onyx-release`, after CI publishes the release, using the same key as the tag (`.github/allowed_signers`).

onyx carries the allowed signers in the binary (from `.github/allowed_signers` at build time) and installs nothing whose `.sha256` isn't signed by one of them.

## Requirements

### `onyx update`
- **SE-01** onyx finds the newest release from where `https://github.com/sumdahl/onyx/releases/latest` redirects (`curl --head`, not followed), then downloads its tarball, `.sha256` and `.sha256.sig` with `curl` into a fresh folder of its own (mode 0700, created new, never reused) in the system temp folder, so a dry run leaves the home untouched. Only `https://github.com/sumdahl/onyx/releases/...` URLs are asked for; `curl` gets `--proto =https --proto-redir =https --fail` and a 100 MiB cap. Only the `x86_64-unknown-linux-musl` build is published, so another machine is told there's no release for it.
- **SE-02** It verifies, in order, and stops at the first failure with nothing replaced: the signature of the `.sha256` against the built-in allowed signers (`ssh-keygen -Y verify -n onyx-release`); that the `.sha256` names exactly the downloaded tarball; `sha256sum --check`; that every entry `tar` lists is a regular file or folder inside the release's one top folder (no links, no absolute paths, no `..`), and that the folder holds a regular file `onyx`; and that the new binary runs `--version` and prints the release's version.
- **SE-03** The dry run (always shown first, P-14) lists the current and newest versions, the exact URLs, each verification step and its result, and what would be replaced. With nothing newer, it says so and changes nothing.
- **SE-04** onyx replaces only its own running binary (`/proc/self/exe`), and only when that file and its folder belong to you and are under your home. Otherwise it names the package manager or path to update with and exits 1, having changed nothing.
- **SE-05** The swap goes through safe delete (P-01): the new binary is copied beside the old one, then the two are exchanged in one `renameat2(RENAME_EXCHANGE)` in the same folder, so onyx is never missing or half-written. The old binary, now under the temporary name, goes to the trash, so the update can be undone. A filesystem that can't exchange is reported, and nothing is replaced.
- **SE-06** The download folder is removed through safe delete when onyx is done, whatever happened, without a log entry: it's onyx's own scratch, and a dry run writes no log (SD-15). The swap is in the operation log as a command entry, and the old binary's move to the trash as an item entry.
- **SE-09** In debug builds only (the ones tests use), `ONYX_TEST_ALLOWED_SIGNERS` names an allowed-signers file to use instead of the built-in one, so tests can sign a fake release with a test key. Release builds don't read it.

### `onyx remove`
- **SE-07** `onyx remove` lists, with sizes, onyx's binary (under SE-04's rule), `~/.config/onyx`, `$XDG_CACHE_HOME/onyx`, and `$XDG_STATE_HOME/onyx` except the operation log, which stays unless `--with-log` is given. It also lists the completion files from README's paths, if present.
- **SE-08** Everything goes to the trash through safe delete after the dry run and one question, default no. `--permanent` deletes instead. It ends by saying what was kept, and that onyx is gone.

## Test strategy

- Every network and tool step runs through fakes on a temp `$PATH` (`curl` copying from a temp "release" folder, a real `ssh-keygen` with a test key), and the "running binary" is a copy of onyx in the temp home. No test reaches the network or replaces the real onyx.
- SE-02: a bad signature, a wrong checksum, a tarball with two files, a symlink or `../onyx`, and a binary printing the wrong version each stop with nothing replaced.
- SE-04: a binary outside the temp home, or not yours, is refused.
- SE-05: after an update, the copy is the new binary and the old one is in the trash.
- P-05: `--dry-run` and "no" leave the temp home unchanged except for the cleaned-up download folder.
- SE-07, SE-08: `remove` trashes the listed folders and the copy and keeps the log; `--with-log` takes it too.

## Decisions

- **The repository goes public** before `onyx update` is used for real (maintainer, 2026-10-30), so `curl` can see releases. Until then `onyx update` reports that it can't find a release.
- **The maintainer signs each release's checksum** (maintainer, 2026-10-30): after CI publishes, `ssh-keygen -Y sign -f ~/.ssh/id_personal -n onyx-release <tarball>.sha256`, then `gh release upload vX.Y.Z <tarball>.sha256.sig`. AGENTS.md's release steps say so. `.github/allowed_signers` allows the key for both the `git` and `onyx-release` namespaces.

- **Owning tools for the network and crypto**, as P-07 and the no-dependencies rule ask. `curl`, `sha256sum` and `ssh-keygen` are on nearly every Linux system.
- **The maintainer signs the checksum, not CI.** The signing key never leaves the maintainer's machine.
- **Only a binary you own in your home is replaced.** A package manager's file is the package manager's.
