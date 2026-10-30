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
| 001 | [Safe delete](001-safe-delete.md) | SD | Draft |
