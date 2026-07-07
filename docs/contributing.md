# Contributing

## Prerequisites

- Rust 2021 edition (stable toolchain)
- Linux or Windows development environment
- Git

## Building

```bash
# Debug build
cargo build

# Release build (optimised)
cargo build --release

# Check compilation without producing binaries
cargo check
```

## Testing

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run a specific test
cargo test test_name
```

## Code Quality

```bash
# Lint checks
cargo clippy

# Format check
cargo fmt --check

# Auto-format
cargo fmt

# Fix some warnings automatically
cargo fix
```

## Project Structure

```
src/
├── main.rs              # Entry point
├── cli/                 # CLI argument parsing (clap)
│   ├── parse.rs         # Command and subcommand definitions
│   ├── help.rs          # Custom help handler
│   └── version.rs       # Version display
├── commands/            # Command business logic
│   ├── draft.rs         # baller draft   (install)
│   ├── eject.rs         # baller eject   (uninstall)
│   ├── roster.rs        # baller roster  (list/search)
│   ├── freeze.rs        # baller freeze  (pin/unpin)
│   ├── substitute.rs    # baller substitute (swap)
│   ├── update.rs        # baller update  (upgrade)
│   ├── sweep.rs         # baller sweep   (clean cache)
│   └── build.rs         # baller build   (from manifest)
├── config/
│   └── config.rs        # Configuration parsing
├── core/                # Core engine modules
│   ├── package.rs       # Package data model
│   ├── registry.rs      # Multi-source registry client
│   ├── downloader.rs    # Download, verify, extract
│   ├── db.rs            # SQLite database
│   ├── dep_solver.rs    # Dependency resolution
│   ├── hooks.rs         # Pre/post hook system
│   └── manifest.rs      # Manifest parsing
├── context.rs           # Shared app context
├── http/                # HTTP registry clients
│   ├── github.rs        # GitHub Releases
│   ├── registry_api.rs  # Baller registry
│   └── chocolatey.rs    # Chocolatey/NuGet
├── platform/            # OS abstraction
│   ├── common.rs        # PlatformManager trait
│   ├── linux.rs         # Linux implementation
│   └── windows.rs       # Windows implementation
└── error/
    └── error.rs         # Error types
```

## Code Conventions

- Non-OS structs: PascalCase with context (`BallerConfig`, not `Config`)
- Variables: explicit type, snake_case
- Booleans: prefix with `is_`, `has_`, `can_`, `should_`
- Collections: plural names
- Propagate errors with `?` up to `main()`
- OS-specific code behind `#[cfg(target_os = "...")]` guards

## Config File Format

Baller reads `~/.baller/baller.conf` with INI-style sections:

```ini
[baller]
install_dir = /home/user/.local/bin
cache_dir = /home/user/.baller/cache
db_path = /home/user/.baller/db/baller.db

[registry]
source_order = github,baller,chocolatey
baller_registry_url = https://registry.baller.dev/api
chocolatey_feed_url = https://community.chocolatey.org/api/v2
github_enabled = true
baller_enabled = true
chocolatey_enabled = true

[hooks]
pre_install = true
post_install = true
pre_eject = true
post_eject = true
pre_update = true
post_update = true
```

## Dependencies

Key crates used:

| Crate | Purpose |
|---|---|
| `clap` (derive) | CLI argument parsing |
| `reqwest` (blocking) | HTTP client |
| `rusqlite` (bundled) | SQLite database |
| `serde` / `serde_json` | Serialization |
| `sha2` | SHA-256 verification |
| `semver` | Version comparison |
| `indicatif` | Progress bars |
| `colored` | Terminal output |
| `zip` / `tar` / `flate2` / `bzip2` / `xz2` | Archive extraction |
