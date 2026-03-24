# Changelog

## Unreleased (Post v0.1 baseline)

### Added

- End-to-end `show` and `movie` organization flow tests.
- End-to-end `scan --json --output` integration test.
- Scan JSON file export via `--output` (requires `--json`).
- Scan filtering options: `--only-failed`, `--min-confidence`.
- Scan JSON metadata for active filters and omitted-item counts.
- Rich conflict diagnostics with typed categories and blocker hints.

### Improved

- Destination preflight checks now detect read-only path issues and parent-path file collisions.
- Conflict presentation now groups and labels conflict kinds for easier resolution.
- JSON scan items now include source path and extension details.
- Metadata lookup fallback is resilient to TMDB errors.

### Tooling

- Rust formatting and linting components installed (`rustfmt`, `clippy`).
- Codebase is formatted and clippy-clean under strict settings (`-D warnings`).
