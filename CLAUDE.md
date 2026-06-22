# Project Instructions

## Requirements Tracking

`REQUIREMENTS.md` is the single source of truth for project progress. After any code change:

1. Update the **Status** column for affected FR/NFR IDs. Format: `[DONE](src/file.rs#L42)`, `[PARTIAL](src/file.rs)`, `NOT STARTED`, `BACKLOG`. The status word is the link text; the href points to the implementing source.
2. If a feature is deferred, add it to the **Backlog** table (chapter 4) with a BL-ID and link back to the originating requirement.
3. Every requirement MUST have a corresponding test in `tests/requirements.rs`. Unimplemented requirements have tests that call `todo!()` — they fail, which is expected until the feature is built.
4. Backlog items must originate from a requirement. No "Enhancement" items without an FR/NFR.

## Testing

- `cargo test` shows all requirements as an ascending pass/fail list
- Implemented features pass; unimplemented features fail with `todo!()`
- When implementing a feature, its test turns green — no manual test updates needed

## Code Style

- No asset-specific names (robot models, tool names) in source code — only in test fixture paths
- Requirement IDs as single-line comments above the implementing function: `// FR-020, FR-021a`
- No multi-line doc comments — one short comment max
