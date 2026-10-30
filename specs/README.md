# Specs

molt is built spec-first. Code exists to satisfy a spec. Every behaviour a user can observe is written here before it is built.

## Lifecycle

`Draft` → `Accepted` → `Implemented`

1. **Draft:** write the spec using the template below. Open questions go in the spec, not in chat.
2. **Accepted:** the spec has been reviewed (grill it), open questions are resolved, and the requirements are frozen. Changing an accepted requirement means editing the spec in the same PR as the code.
3. **Implemented:** every requirement has at least one test that names its ID, and all of those tests pass.

## Requirement IDs

Each spec has a prefix (`P`, `SD`, `RE`, `CL`…). Requirements are numbered `PREFIX-NN` and never renumbered. A removed requirement is struck through, not deleted.

Tests name the requirement they prove:

```rust
#[test]
fn sd_03_symlink_target_is_never_touched() { … }
```

`grep -ri 'sd_03' tests src` must find the proof for SD-03.

## Template

```markdown
# NNN: Title

Status: Draft | Accepted | Implemented
Prefix: XX

## Goal
One paragraph: what the user gets.

## Non-goals
What this spec deliberately leaves out.

## Requirements
- **XX-01** Given …, when …, then ….

## Open questions
- …
```

## Index

| # | Spec | Prefix | Status |
|---|---|---|---|
| 000 | [Principles](000-principles.md) | P | Accepted |
| 001 | [Safe delete](001-safe-delete.md) | SD | Implemented |
| 002 | [Rules engine](002-rules-engine.md) | RE | Implemented |
| 003 | [Config and whitelist](003-config.md) | CF | Implemented |
| 004 | [CLI skeleton](004-cli.md) | CL | Implemented |
| 005 | [`molt scan`](005-scan.md) | SC | Implemented |
| 006 | [`molt clean`](006-clean.md) | CN | Implemented |
| 007 | [`molt system`](007-system.md) | SY | Implemented |
| 008 | [`molt purge`](008-purge.md) | PG | Implemented |
| 009 | [`molt installer`](009-installer.md) | IN | Implemented |
| 010 | [Built-in rules](010-built-in-rules.md) | BR | Implemented |
| 011 | [Locked folders](011-locked-folders.md) | LF | Implemented |
| 012 | [`molt analyze`](012-analyze.md) | AN | Implemented |
| 013 | [`molt status`](013-status.md) | ST | Implemented |
| 014 | [`molt uninstall`](014-uninstall.md) | UN | Implemented |
| 015 | [`molt optimize`](015-optimize.md) | OP | Implemented |
| 016 | [The molt banner](016-banner.md) | BN | Implemented |
| 017 | [`molt history`](017-history.md) | HI | Implemented |
| 018 | [Shell completions](018-completions.md) | CO | Implemented |
| 019 | [The `molt` menu](019-menu.md) | MN | Implemented |
