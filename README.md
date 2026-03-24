# organize

A Rust CLI tool that organizes downloaded TV and movie files into media-server friendly folder structures.

## Features

- Show and movie organization commands
- Scan mode for metadata preview
- Scan JSON mode for automation and tooling
- Parser handles common release-name patterns
- Optional TMDB year lookup
- Dry-run support with conflict preview
- Conflict policy controls: `skip`, `overwrite`, `abort`
- Multiple operation modes: move, copy, hardlink, symlink
- Config-driven defaults with CLI overrides

## Quick Start

```bash
cargo run -- show ./downloads/My.Show.Complete ./media/tv --dry-run
cargo run -- movie ./downloads/My.Movie.2023 ./media/movies --dry-run
cargo run -- scan ./downloads/My.Show.Complete --type show
cargo run -- scan ./downloads/My.Show.Complete --type show --json
```

## Command Overview

```text
organize <COMMAND> [OPTIONS]

COMMANDS:
  show   Organize TV show files
  movie  Organize movie files
  scan   Parse and display detected metadata
```

Use `--help` at each level:

```bash
cargo run -- --help
cargo run -- show --help
cargo run -- movie --help
cargo run -- scan --help
```

## Scan Output

`scan` supports two output styles:

- Human-readable default output
- Machine-readable JSON output with `--json`

JSON item records include parse metadata plus source details (`file_name`, `source_path`, `extension`, title/year/season/episode, detected kind, confidence).

Example:

```bash
cargo run -- scan ./downloads/My.Show.Complete --json
cargo run -- scan ./downloads/My.Show.Complete --json --output ./reports/scan.json
cargo run -- scan ./downloads/My.Show.Complete --only-failed
cargo run -- scan ./downloads/My.Show.Complete --min-confidence medium
```

## Conflict Handling

For `show` and `movie` commands:

- `--on-conflict skip` (default): leave existing destination files untouched
- `--on-conflict overwrite`: replace existing destination files
- `--on-conflict abort`: fail before execution if any conflicts exist

Compatibility flag:

- `--overwrite` is still supported and forces overwrite behavior

## Configuration

Config load order (later wins):

1. Global config (`$XDG_CONFIG_HOME/organize/config.toml`)
2. Local config (`.organize.toml`)
3. Explicit config (`--config <PATH>`)
4. CLI flags

Example config is provided in `.organize.toml.example`.

## Development

### Prerequisites

- Rust stable toolchain
- Recommended components:

```bash
rustup component add rustfmt
rustup component add clippy
```

### Useful Commands

```bash
cargo check
cargo test
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
```

### Project Layout

```text
src/
  cli.rs
  config.rs
  executor.rs
  logging.rs
  main.rs
  planner.rs
  prompt.rs
  scanner.rs
  tmdb.rs
  parser/
    mod.rs
    show.rs
    movie.rs
    tokens.rs
```

## Notes

- Files are not renamed; only organized into folder structures.
- The tool is optimized for iterative, test-driven development and Copilot-assisted workflows.
