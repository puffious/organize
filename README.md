![wait, it's all AI? always has been](docs/ai.jpg)

# organize

A Rust CLI tool that organizes downloaded TV and movie files into media-server friendly folder structures.

## Features

- Show and movie organization commands
- `doctor` command for config, path, and environment checks
- Scan mode for metadata preview
- Scan JSON mode for automation and tooling
- Rich scan diagnostics with parser source details and issue summaries
- Parser handles common release-name patterns
- Optional TMDB year lookup
- Dry-run support with conflict preview
- Conflict policy controls: `skip`, `overwrite`, `abort`
- Multiple operation modes: move, copy, hardlink, symlink
- Config-driven defaults with CLI overrides
- Docker support for running without installing the binary locally

## Quick Start

```bash
organize show ./downloads/My.Show.Complete ./media/tv --dry-run
organize movie ./downloads/My.Movie.2023 ./media/movies --dry-run
organize scan ./downloads/My.Show.Complete --type show
organize scan ./downloads/My.Show.Complete --type show --json
```

## Command Overview

```text
organize <COMMAND> [OPTIONS]

COMMANDS:
  show    Organize TV show files
  movie   Organize movie files
  scan    Parse and display detected metadata
  doctor  Validate config, paths, and runtime setup
```

Use `--help` at each level:

```bash
organize --help
organize show --help
organize movie --help
organize scan --help
organize doctor --help
```

## Scan Output

`scan` supports two output styles:

- Human-readable default output
- Machine-readable JSON output with `--json`

JSON output includes filter metadata, omitted counts, diagnostics summaries, parse confidence, detected kind, parser mode, and field source details (`title_source`, `year_source`, `season_source`, `episode_source`).

Use `-v` with text output to show parser source details and issue summaries for each item.

Example:

```bash
organize scan ./downloads/My.Show.Complete --json
organize scan ./downloads/My.Show.Complete --json --output ./reports/scan.json
organize scan ./downloads/My.Show.Complete --only-failed
organize scan ./downloads/My.Show.Complete --min-confidence medium
organize -v scan ./downloads/My.Show.Complete
```

## Doctor Command

`doctor` is a read-only setup check for config discovery, TMDB key presence, source path validity, destination path readiness, and effective media extension configuration.

Example:

```bash
organize doctor
organize doctor --source ./downloads/My.Show.Complete --destination ./media/tv
organize doctor --json
organize doctor --json --output ./reports/doctor.json
```

Use this before a large run when you want to confirm the tool sees the config and paths you expect.

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

## Docker

Build the image:

```bash
docker build -t organize .
```

Run commands by bind-mounting your media folders and config file. Using an explicit `--config` path is the clearest container setup.

```bash
docker run --rm \
  -v "$PWD/.organize.toml:/config/config.toml:ro" \
  -v "$HOME/Downloads:/source" \
  -v "$HOME/Media:/dest" \
  organize doctor --config /config/config.toml --source /source --destination /dest


docker run --rm \
  -v "$PWD/.organize.toml:/config/config.toml:ro" \
  -v "$HOME/Downloads:/source" \
  -v "$HOME/Media:/dest" \
  organize show --config /config/config.toml --dry-run /source /dest
```

For real non-interactive runs inside a container, pass `--yes`.

```bash
docker run --rm \
  -v "$PWD/.organize.toml:/config/config.toml:ro" \
  -v "$HOME/Downloads:/source" \
  -v "$HOME/Media:/dest" \
  organize show --config /config/config.toml --yes /source /dest
```

Notes:

- `move` changes files in the mounted source path, so start with `--dry-run`
- hardlinks may fail across mounts or filesystems
- symlinks can be awkward when host and container paths differ
- mount the exact folders you want the container to see

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

### CI

This repository includes GitHub Actions CI in .github/workflows/ci.yml with:

- rustfmt check
- clippy with warnings denied
- full test suite (unit + integration)

CI runs on pushes to main and on pull requests.

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
