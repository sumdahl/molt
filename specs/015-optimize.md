# 015: `molt optimize`

Status: Implemented (OP-01 to OP-08); Draft (OP-09 to OP-14, #49)
Prefix: OP

## Goal

`molt optimize` does safe system upkeep through each tool's own command, the way `molt system` (007) frees space: it shows everything first, asks once, and runs each command through `sudo`. It covers packages nothing needs any more (#48) and maintenance tasks (#49). Read-only health checks (#50) add requirements here later.

## Non-goals

- Deciding what's an orphan. The package manager decides; molt shows its answer and passes it back.
- Removing anything the preview didn't name (P-14).
- Flatpak's unused runtimes, for now. `flatpak uninstall --unused` has no read-only way to list what it would remove, so it can't be previewed (P-14). It comes back when one is verified.
- Running as root (CL-07).

## Model

1. Start up (004). Root is refused (CL-07).
2. **Detect** (P-10): a task is active only when its tools are found on the absolute `$PATH` entries (SC-10), and `sudo` too (SY-02).
3. **Preview** (P-14): each active task runs its read-only query without `sudo` and shows the packages it found, and the exact live command.
4. **Ask** (P-06): one yes/no for all of it, default no.
5. **Act**: run each task's live command through `sudo`, naming exactly the packages the preview listed. The manager still asks its own question.
6. **Report** each command's exit status. Log every command (SY-11).

## Tasks

| id | Active when found | Preview (no sudo): lists the packages | Live command |
|---|---|---|---|
| `pacman-orphans` | `pacman` | `pacman -Qdtq` (exit 1 with no output means none) | `pacman -Rns <listed packages>` |
| `apt-autoremove` | `apt-get` | `apt-get -s autoremove` (the `Remv <pkg>` lines) | `apt-get autoremove` |
| `dnf-autoremove` | `dnf` | `dnf repoquery --unneeded --qf "%{name} "` (a space after each name, as dnf 4 adds a newline and dnf 5 doesn't) | `dnf remove <listed packages>` |
| `zypper-unneeded` | `zypper` | `zypper --quiet packages --unneeded` (the name column) | `zypper remove --clean-deps <listed packages>` |

- `pacman -Qdtq` lists packages installed as dependencies that nothing needs. `-Rns` also removes their own dependencies that would then be orphans, and pacman lists those before it asks.
- `apt-get autoremove` takes no package names; its list is apt's own, and the preview shows that same list.

## Maintenance tasks

| id | Active when found | Preview (no sudo) | Live command |
|---|---|---|---|
| `fstrim` | `fstrim`, `sudo` | none: `fstrim --dry-run` sees no filesystems without root | `sudo -- fstrim --all --verbose` |
| `font-cache` | `fc-cache` | none | `fc-cache`, as you, **without** `sudo` |
| `dns-cache` | `resolvectl`, `sudo` | `resolvectl statistics`: its "Current Cache Size" line | `sudo -- resolvectl flush-caches` |

- `fstrim --all` tells each mounted filesystem whose device supports it which blocks no file uses (TRIM). No file's data changes.
- `fc-cache` without `-f` rebuilds only your font caches that are out of date.
- Flushing the DNS cache drops remembered answers; the next lookups ask again.

## Requirements

- **OP-01** `molt optimize` lists every active task. `--only <id>…` and `--skip <id>…` narrow the list; an unknown id is a usage error (exit 2), as SY-01.
- **OP-02** A task whose tools aren't found on an absolute `$PATH` entry is inactive and reported as such (SY-02), and so is every task that needs `sudo` when `sudo` isn't found.
- **OP-03** Every run shows the preview first (P-14): each task, the packages its query listed (or "nothing to remove"), and the exact live command with the absolute `sudo` and tool paths. Queries run as SY-04 does: no `sudo`, no shell, stdin closed, `$PATH` limited to its absolute entries.
- **OP-04** A package name is used only if it's one plain name (UN-08's check: no leading `-`, no spaces or shell characters). If any listed name fails it, the task is left out and says so.
- **OP-05** A task whose query lists nothing, or fails, runs nothing. A failed query is reported with its exit status.
- **OP-06** The live command names exactly the packages the preview listed (for apt, apt's own list). It runs as `sudo -- <tool path> <args>` (SY-07) without `--noconfirm` or `-y`, so the manager shows what it will remove and asks again. `--yes` skips only molt's question.
- **OP-07** A dry run (`--dry-run`, the config, or `MOLT_DRY_RUN`) stops after the preview and never runs `sudo` (SY-05). Without a terminal, a live run needs `--yes`, as SY-06.
- **OP-08** Tasks run in the table's order; one failing doesn't stop the next (SY-08). The result lists each command and its exit status. The exit code is 0 when all succeeded, 3 otherwise (P-13). Every command is in the operation log (SY-11). `--json` prints the preview and the result as `molt system` does.

### Maintenance (#49)
- **OP-09** The maintenance tasks come after the package tasks, in the table's order, with the same `--only`, `--skip`, question, dry run, log, result and exit codes (OP-01, OP-07, OP-08).
- **OP-10** The preview shows each one's name, what it does in one line, its preview's line if it has one, and the exact live command with absolute paths (P-14). A dry run runs only the preview commands.
- **OP-11** `font-cache` runs as you, never through `sudo`, and is active without `sudo` (P-04).
- **OP-12** `dns-cache` is active only when `resolvectl statistics` succeeds, as that means systemd-resolved is running; otherwise it's shown as left out with the reason. Its preview line is the "Current Cache Size" line.
- **OP-13** No maintenance command gets `--force` or `-f`, and none takes a path from molt: `fstrim --all` chooses its own filesystems.
- **OP-14** The live commands run after the package removals; a failing one doesn't stop the rest (OP-08).

## Test strategy

- Smoke tests with fake `sudo`, `pacman`, `apt-get`, `dnf` and `zypper` scripts in a temp dir that record their arguments, as in 007's and 014's tests. No test runs a real one.
- OP-07 / P-05: a dry run records only queries and no `sudo`.
- OP-06: the recorded `sudo` line is `-- <bin>/pacman -Rns a b` for a fake that listed `a` and `b`, with no `--noconfirm`.
- OP-04: a fake listing `-Rdd` or `a b` leaves the task out.
- OP-05: pacman's exit 1 with no output shows "nothing to remove" and runs nothing.
- OP-08: a failing fake removal exits 3 and the next task still runs.
- OP-09 to OP-14: fake `fstrim`, `fc-cache` and `resolvectl`. A dry run records only `resolvectl statistics`. `--yes` records `sudo -- <bin>/fstrim --all --verbose`, a bare `fc-cache` with no `sudo`, and `sudo -- <bin>/resolvectl flush-caches`. Without `sudo`, only `font-cache` is active. A failing `resolvectl statistics` leaves `dns-cache` out.

## Decisions

- **A new command, not a `molt system` task.** `system` frees space and measures it; optimize's tasks are upkeep, and #49 and #50 add tasks that free nothing.
- **Names passed explicitly** where the manager takes them, so the live run can't remove more top-level packages than the preview showed, even if the system changed in between.
- **Two yeses**, as `molt uninstall`: molt's, then the manager's. The maintenance tools don't ask again; molt's question is the only one, which is fine as none of them removes a file.
- **No fstrim preview.** molt could list discard-capable filesystems itself, but `fstrim --all` decides on its own, and a list that might differ from what runs would be worse than none.
