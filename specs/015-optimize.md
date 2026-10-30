# 015: `molt optimize`

Status: Accepted (#48)
Prefix: OP

## Goal

`molt optimize` does safe system upkeep through each tool's own command, the way `molt system` (007) frees space: it shows everything first, asks once, and runs each command through `sudo`. This spec starts with packages nothing needs any more (#48). Maintenance tasks (#49) and read-only health checks (#50) add requirements here later.

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
| `dnf-autoremove` | `dnf` | `dnf repoquery --unneeded --qf %{name}` | `dnf remove <listed packages>` |
| `zypper-unneeded` | `zypper` | `zypper --quiet packages --unneeded` (the name column) | `zypper remove --clean-deps <listed packages>` |

- `pacman -Qdtq` lists packages installed as dependencies that nothing needs. `-Rns` also removes their own dependencies that would then be orphans, and pacman lists those before it asks.
- `apt-get autoremove` takes no package names; its list is apt's own, and the preview shows that same list.

## Requirements

- **OP-01** `molt optimize` lists every active task. `--only <id>…` and `--skip <id>…` narrow the list; an unknown id is a usage error (exit 2), as SY-01.
- **OP-02** A task whose tools or `sudo` aren't found on an absolute `$PATH` entry is inactive and reported as such (SY-02).
- **OP-03** Every run shows the preview first (P-14): each task, the packages its query listed (or "nothing to remove"), and the exact live command with the absolute `sudo` and tool paths. Queries run as SY-04 does: no `sudo`, no shell, stdin closed, `$PATH` limited to its absolute entries.
- **OP-04** A package name is used only if it's one plain name (UN-08's check: no leading `-`, no spaces or shell characters). If any listed name fails it, the task is left out and says so.
- **OP-05** A task whose query lists nothing, or fails, runs nothing. A failed query is reported with its exit status.
- **OP-06** The live command names exactly the packages the preview listed (for apt, apt's own list). It runs as `sudo -- <tool path> <args>` (SY-07) without `--noconfirm` or `-y`, so the manager shows what it will remove and asks again. `--yes` skips only molt's question.
- **OP-07** A dry run (`--dry-run`, the config, or `MOLT_DRY_RUN`) stops after the preview and never runs `sudo` (SY-05). Without a terminal, a live run needs `--yes`, as SY-06.
- **OP-08** Tasks run in the table's order; one failing doesn't stop the next (SY-08). The result lists each command and its exit status. The exit code is 0 when all succeeded, 3 otherwise (P-13). Every command is in the operation log (SY-11). `--json` prints the preview and the result as `molt system` does.

## Test strategy

- Smoke tests with fake `sudo`, `pacman`, `apt-get`, `dnf` and `zypper` scripts in a temp dir that record their arguments, as in 007's and 014's tests. No test runs a real one.
- OP-07 / P-05: a dry run records only queries and no `sudo`.
- OP-06: the recorded `sudo` line is `-- <bin>/pacman -Rns a b` for a fake that listed `a` and `b`, with no `--noconfirm`.
- OP-04: a fake listing `-Rdd` or `a b` leaves the task out.
- OP-05: pacman's exit 1 with no output shows "nothing to remove" and runs nothing.
- OP-08: a failing fake removal exits 3 and the next task still runs.

## Decisions

- **A new command, not a `molt system` task.** `system` frees space and measures it; optimize's tasks are upkeep, and #49 and #50 add tasks that free nothing.
- **Names passed explicitly** where the manager takes them, so the live run can't remove more top-level packages than the preview showed, even if the system changed in between.
- **Two yeses**, as `molt uninstall`: molt's, then the manager's.
