# Agent Loop Prompt

## Context
- Specs: `specs/*` — read before implementing
- Code: `src/*` — search before assuming unimplemented
- Branch: main — do not create/switch branches
- Tasks: `br` (beads rust) — single source of truth
- Progress: Check `progress.txt` on startup for previous iteration learnings

## Workflow

1. **Get task**: `br ready --json | jq '.[0]'` → choose highest priority
2. **Research**: Use ≤5 parallel `task` tool calls with explore/general agents for search/read
3. **Implement**: No stubs/placeholders. Complete implementations only.
4. **Test**: Run tests (find command in project) — fix all failures including unrelated ones
5. **Commit**: `git add -A && git commit -m "type: desc"` — NO push
6. **Tag**: If tests pass and no errors, create git tag (increment from last or start 0.0.1)
7. **Update tasks**: `br close <id> --reason "summary"` or `br update <id> --status in_progress`

## Subagent Strategy

| Task | Agent Type | Count |
|------|-----------|-------|
| Search/read code | explore | ≤5 parallel |
| Build/test | general | 1 only |
| Debug/architecture | general (multiple iterations) | as needed |
| Spec inconsistencies | general + thorough research | as needed |

## Critical Rules

- **Never assume — verify**: Do NOT assume functionality exists or doesn't exist. Always search the source code first using `glob` and `grep` before implementing or claiming something is missing.
- **br is truth**: All tasks, discoveries, learnings → `br create`, `br comments add`, `br update`
- **Fix all failures**: Even unrelated test failures are your responsibility
- **progress.txt**: Read at startup for previous iteration learnings
- **Capture the why**: Documentation explains importance, not just what
- **Single source of truth**: No adapters or migration layers

## br Commands Reference

```bash
# Get next task
br ready                          # List ready work
br ready --json                   # Machine-readable output

# Work on task
br update <id> --status in_progress  # Claim work
br close <id> --reason "Done"       # Complete task
br comments add <id> "Note"          # Add discovery/learning

# Create new tasks
br create "Title" --type feature --priority 1
br create "Title" --type bug --priority 0
```

## Output Contract

End every response with exactly:

```
DONE|<summary max 50 chars>|<learning max 80 chars>
```

or

```
BLOCKED|<reason max 50 chars>|<what you tried max 80 chars>
```

## Tools Reference

- `read` — Read file contents
- `write` — Create/overwrite files
- `edit` — Edit existing files
- `bash` — Run commands (git, cargo, br, etc.)
- `grep` — Search file contents (via bash)
- `find`/`ls` — Find files (via bash)

## Testing

Before finishing any task:
1. Run tests with `cargo test` (Rust project)
2. Fix ALL test failures
3. Run `cargo check` and `cargo clippy` for linting
