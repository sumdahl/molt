# 000: Principles

Status: Accepted
Prefix: P

## Goal

These rules hold for every command molt has or will have. A feature spec may add restrictions. It may not relax these rules.

## Requirements

- **P-01 One way to delete.** Every removal, move-to-trash, or rename of user data goes through the safe-delete module ([001](001-safe-delete.md)). `clippy.toml` forbids `std::fs` removal calls everywhere else, and CI runs clippy with `-D warnings`.
- **P-02 Prove it's safe, or leave it.** If molt cannot show that a path is safe to remove, it skips the path and reports the reason. Unknown means no.
- **P-03 Fail closed.** If config, whitelist, or rule files exist but can't be read or parsed, molt stops before deleting anything.
- **P-04 No root by default.** molt runs as the invoking user. System-level actions (pacman cache, journal, coredumps) run only through an explicit step that names the privileged command and asks for confirmation. If molt is started as root without that step, it refuses to run.
- **P-05 Dry run everywhere.** Every command accepts `--dry-run`, a global flag, so adding it is always safe. A dry run:
  - changes nothing anywhere. It deletes, trashes, moves and creates nothing: no trash directory, no operation-log entry, no config or state file.
  - never runs an owning tool's cleanup command (P-07). It shows the exact command a live run would run, or runs the tool's own read-only preview if it has one (`paccache -d`).
  - never asks for privileges. There is no `sudo` prompt (P-04).
  - shows the same plan, sizes and exit code as a live run, with each outcome marked as what *would* happen.
  - is accepted by read-only commands (`scan`, `analyze`, `status`) too, which behave exactly as they do without it.

  Every command spec must include a requirement, with a test, that its dry run leaves a snapshot of the fake home unchanged and starts no mutating external command.
- **P-06 Confirm before mutating.** Interactive runs show what will be removed, with sizes, and wait for confirmation. Non-interactive runs require `--yes`.
- **P-14 Dry run first, always.** Every operation that changes anything runs as a dry run first, and a live run only carries out what that dry run showed.
  - A live run starts with a dry run of the same plan and shows its result: every item, its size, and what would happen to it. Only then does it ask (P-06) and act.
  - `--yes` skips the question, never the preview. The preview is printed before anything changes, in `--json` too.
  - The live run acts only on items the preview showed. Anything that changed in between is skipped (SD-08, SD-10), never added.
  - This holds for every kind of change: removing, trashing, changing permissions (011), and running an owning tool's command (P-07), whose exact command line is shown first.
  - Each command spec must include a test that its live run prints the preview before the first change.
- **P-07 Ask the owning tool first.** When a tool has its own cleanup command (`paccache`, `npm cache clean`, `docker system prune`, `journalctl --vacuum-*`, `flatpak uninstall --unused`, `uv cache clean`…), molt runs that command instead of deleting the tool's files directly.
- **P-08 Don't clean what's in use.** A rule may name the processes that own its data. molt skips the rule while any of those processes is running.
- **P-09 Log every change.** Every mutation is appended to `$XDG_STATE_HOME/molt/operations.log` (one JSON object per line). `MOLT_NO_OPLOG=1` disables the log.
- **P-10 Any Linux, not just Omarchy.** Core behaviour uses only the Linux kernel ABI, XDG base directories, and the FreeDesktop trash spec. Distro and package-manager rules turn on only when their tool is detected.
- **P-11 Machine-readable output.** Every command that reports something supports `--json`, and that output is a stable interface.
- **P-12 Honest numbers.** Reported sizes are on-disk sizes (`st_blocks × 512`). Hard links are counted once. A partial scan says that it is partial.
- **P-13 Exit codes.** `0` means success. `1` means a fatal error with nothing changed. `2` means bad usage. `3` means partial success: some items failed and each one is reported.

## Decisions

- **Trash vs. permanent delete.** Moving data to the trash frees no space until the trash is emptied, so the default depends on what kind of data it is:
  - Regenerable data (caches, build artifacts, logs) is deleted permanently by default.
  - User content (installers in Downloads, items picked in `analyze`, app data during `uninstall`) goes to the trash by default.
  - `--trash` and `--permanent` override the default. Each rule declares its data kind.
