# 011: Locked folders

Status: Implemented
Prefix: LF

## Goal

Some tools leave folders behind that their owner can't read. makepkg leaves `pkg` folders with mode `d--x--x--x` in AUR build caches, which locked 33 GB of `~/.cache/yay` on the maintainer's laptop. Without help, neither onyx nor `rm -rf` can remove them. onyx finds these folders, shows them in every dry run and scan, and opens them up only after the user agrees, then removes them like everything else.

## Non-goals

- Folders owned by someone else. Changing those needs root, and they stay a failure (P-04).
- Files that can't be read. Removing a file needs write permission on its folder, not read permission on the file, so they don't block removal.

## Model

A **locked folder** is a directory inside a candidate that safe delete can't open for listing (`EACCES`), whose owner is the effective uid.

A plan carries a **permission choice**: `keep` (the default) or `fix-owned`. Only `fix-owned` lets safe delete change permissions, and only an explicit yes from the user sets it (LF-05).

## Requirements

### Finding
- **LF-01** When safe delete meets a locked folder, it keeps the folder and everything inside it, carries on with the rest of the candidate, and lists the folder in the report. The candidate's outcome is partial, as for a mount (SD-07). This holds in a dry run and in a live run with `keep`. In trash mode the whole candidate is moved, locked folders included (a rename doesn't need to read inside), and the folders are still listed, since their size is unknown.
- **LF-02** A folder that can't be listed and is owned by someone else stops the candidate with `EACCES`, as now (SD-14). It is never changed.

### Fixing
- **LF-03** In a live run with `fix-owned`, safe delete adds `u+rwx` to a locked folder, then removes it and its contents as usual. Group and other bits are never changed.
  - The folder is opened with `O_PATH | O_NOFOLLOW | O_DIRECTORY` relative to its parent's handle, and its identity (`st_dev`, `st_ino`, owner) is checked on that handle. The mode is changed through that handle (`/proc/self/fd/N`), never by path, so a folder swapped for a symlink can't redirect the change (SD-09).
  - If the change fails, the folder is kept and the candidate fails with that error.
- **LF-04** Every folder whose permissions were changed is counted in the candidate's operation-log entry (SD-15).

### Consent
- **LF-05** `fix-owned` is only ever set from the user's explicit consent. A command that would remove candidates with locked folders first shows the folders, says what it would change (`u+rwx`) and why, and asks. The default answer is no.
  - `--yes` alone doesn't give that consent. A non-interactive run needs `--fix-permissions` as well.
  - In a dry run, `--fix-permissions` changes nothing; the report says the folders would be opened and removed.
- **LF-06** The dry run and `onyx scan` show each rule's locked folders: how many, that their contents weren't measured, and that `onyx clean` will ask to open them. `--json` lists their paths. One folder is worded in the singular (#85).

## Test strategy

- Locked folders are made in a temp home with mode `0o111`, as makepkg leaves them. Tests skip as root, where permissions aren't enforced.
- LF-03's swap is tested with the SD-09 hook: swap the locked folder for a symlink to an outside folder before the change, and check the outside folder's mode is unchanged.
- LF-05 is tested on `clean` (006, #6). Here, only that `keep` never changes a mode.

## Decisions

- **Consent is asked once per run**, after the P-14 preview, listing every locked folder.
- **A live run without consent is partial** (LF-01): it removes the rest and keeps the locked folders. A later run with consent finishes the job.
