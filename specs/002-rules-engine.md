# 002: Rules engine

Status: Implemented
Prefix: RE

## Goal

Every cleaning rule is data, not code: a TOML entry that says where some reclaimable data lives, what kind of data it is, and how to clean it properly. The engine loads the built-in rules and the user's rules, checks them strictly, works out which rules apply to this machine, and turns their paths into safe-delete candidates ([001](001-safe-delete.md)). It never deletes anything itself.

## Non-goals

- Sizing, reporting and choosing what to clean. Those belong to `scan` (005) and `clean` (006).
- Running owning-tool commands or checking running processes. The engine only carries those fields; `clean` acts on them (P-07, P-08).
- System-level rules (pacman cache, journal). Those come with the system step (007) and its own anchors.

## Model

A **rule file** holds a list of rules:

```toml
[[rule]]
id = "npm-cache"                          # unique, lowercase, a-z 0-9 and -
name = "npm download cache"
category = "dev-tools"                    # dev-tools | browsers | apps | system-user | trash
kind = "regenerable"                      # regenerable | user-content
paths = ["~/.npm/_cacache"]
clean_with = ["npm", "cache", "clean", "--force"]   # optional (P-07)
processes = ["npm"]                       # optional (P-08)
requires = ["npm"]                        # optional: executables, any one on $PATH (P-10)
```

A **path pattern** starts with an **anchor** (`~`, `$XDG_CACHE_HOME`, `$XDG_CONFIG_HOME`, `$XDG_DATA_HOME`, `$XDG_STATE_HOME`), followed by `/`-separated components. Components may use `*` and `?`, which match within one component only. `*` also matches names that start with `.`.

Matching produces **matches**: `(rule id, candidate, mode)`. The mode comes from the kind: `regenerable` → permanent, `user-content` → trash (000 Decisions).

## Requirements

### Loading
- **RE-01** The built-in rules are compiled into the binary. User rules are read from `$XDG_CONFIG_HOME/onyx/rules/*.toml`, in name order.
- **RE-02** If a user rule file exists but can't be read or parsed, loading fails and onyx stops before anything is deleted (P-03). The error names the file, and the line where there is one.
- **RE-03** Unknown fields, missing required fields, and invalid values (an unknown `kind` or `category`, a bad `id`) are errors, not warnings.
- **RE-04** Two rules with the same `id` are an error, whether both are user rules or one is built in.

### Path patterns
- **RE-05** A pattern must start with an anchor. Absolute paths, `..`, `.`, empty components, and `**` are errors.
- **RE-06** The first component after the anchor must be literal (no `*` or `?`), so a rule can never match all of `~` or all of a base directory.
- **RE-07** Anchors resolve from the context the caller passes in, never from the process environment, so tests use a fake home (as in 001).

### Applicability
- **RE-08** A rule with `requires` applies only when at least one of the listed executables is on `$PATH`. A rule without `requires` always applies. A rule that doesn't apply is reported as inactive, not as an error.

### Matching
- **RE-09** Each pattern is expanded by listing directories, without following any symlink below the anchor. A pattern that matches nothing yields no match and no error. A directory that can't be listed is skipped and reported with reason `unreadable`.
- **RE-10** A match, or a directory on the way to one, that is a symlink is skipped and reported with reason `symlink`. Removing a link frees nothing, and the target is never followed.
- **RE-11** Each match becomes a `Candidate` scanned at match time (SD-08), with the mode its rule's kind implies. A path matched by two rules is reported once, under the first rule in load order.

## Test strategy

- Unit tests parse rule TOML from strings; every error case in RE-02 to RE-06 has a test.
- Matching tests build a fake home in a temp dir, with symlinks pointing out of it.
- A test loads every built-in rule file, so a bad built-in rule fails CI.

## Decisions

- **No overriding built-ins.** A user rule can't replace a built-in rule; a shared `id` is an error (RE-04). Built-ins are switched off by `id` in the config (003, #2).
- **Symlinks are skipped, not removed** (RE-10).
- **No risk level yet.** Add it with `clean` (006, #6) if its picker needs one.
- **Globs stay shallow.** `*` and `?` within one component, no `**`, literal first component (RE-05, RE-06). Deep searches belong to `purge` (008, #8).
