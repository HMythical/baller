# Architecture

## Directory Layout

```
baller/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point, OS check, config + context init
│   ├── cli/                 # CLI parsing with clap derive
│   │   ├── mod.rs
│   │   ├── parse.rs         # BallerCommand, CommandTypes enum
│   │   ├── help.rs          # Custom help output
│   │   └── version.rs       # Version output
│   ├── commands/            # Per-command business logic
│   │   ├── mod.rs
│   │   ├── draft.rs         # Install (with recursive dep resolution)
│   │   ├── eject.rs         # Uninstall
│   │   ├── roster.rs        # List/search installed + remote
│   │   ├── freeze.rs        # Pin/unpin packages
│   │   ├── substitute.rs    # Swap packages
│   │   ├── update.rs        # Update non-frozen packages
│   │   ├── sweep.rs         # Clean cache
│   │   └── build.rs         # Build from manifest (stub)
│   ├── config/
│   │   ├── mod.rs
│   │   └── config.rs        # BallerConfig, RegistryConfig, HooksConfig
│   ├── core/                # Core engine
│   │   ├── mod.rs
│   │   ├── package.rs       # Package / PackageSource structs
│   │   ├── registry.rs      # Multi-source RegistryClient
│   │   ├── downloader.rs    # Download, hash verify, extract
│   │   ├── db.rs            # SQLite state management
│   │   ├── dep_solver.rs    # Dependency resolution, cycle detection
│   │   ├── hooks.rs         # Pre/post hook execution
│   │   └── manifest.rs      # baller.toml/json parser (stub)
│   ├── http/                # HTTP registry clients
│   │   ├── mod.rs           # HttpClient (reqwest wrapper, retries)
│   │   ├── github.rs        # GitHub Releases API
│   │   ├── registry_api.rs  # Baller registry API
│   │   └── chocolatey.rs    # Chocolatey OData v2 feed
│   ├── platform/            # OS abstraction
│   │   ├── mod.rs
│   │   ├── common.rs        # PlatformManager trait
│   │   ├── linux.rs         # Symlink implementation
│   │   └── windows.rs       # Windows implementation
│   ├── error/
│   │   ├── mod.rs
│   │   └── error.rs         # BallError enum
│   ├── context.rs           # AppContext (shared services container)
│   └── utils/
│       ├── mod.rs
│       ├── fs.rs            # File system utilities
│       └── security.rs      # Hash verification utilities
```

## Startup Flow

1. `main()` → `entry()` OS validation (Linux or Windows only)
2. `create_baller_dir()` — ensures `~/.baller/` exists
3. `BallerConfig::parse_config()` — reads `~/.baller/baller.conf`, falls back to defaults
4. `AppContext::new()` — initialises all shared services:
   - Creates `~/.baller/cache/` and `~/.baller/hooks/` dirs
   - `HttpClient` (reqwest with timeouts + retries)
   - `DbManager` (SQLite schema init)
   - `RegistryClient` (source chain respecting per-source enable flags)
   - `Downloader` (cached download, extraction)
5. `BallerCommand::parse_command()` — clap CLI argument parsing
6. `command.execute(&ctx)` — dispatches to the appropriate command module

## Core Data Structures

### BallerConfig
Holds all user-configurable paths (`install_dir`, `db_path`, `cache_dir`, `hooks_dir`),
`RegistryConfig` (source URLs, order, per-source enabled flags), and `HooksConfig` (per-type enable flags).

### AppContext
Lifetime container for all shared services. Created once at startup, passed by reference
to every command function and sub-operation.

### Package
The universal package metadata struct used across the entire system:
- `name`, `version`, `description`, `author`
- `source` (`PackageSource` enum: `GitHub`, `BallerRegistry`, `Chocolatey`)
- `download_url`, `sha256`, `dependencies`

## Platform Abstraction

The `PlatformManager` trait in `platform/common.rs` defines:
- `get_install_dir()` — where binaries go (`~/.local/bin` on Linux)
- `get_config_dir()` — where baller stores data (`~/.baller`)
- `create_symlink(source, name)` — expose a binary in the install dir
- `remove_symlink(name)` — remove a binary from the install dir

Each OS implements the trait in its own module. The `ActiveManager` type alias
resolves to the correct implementation at compile time via `cfg(target_os)`.

## Registry Fallback Chain

The `RegistryClient` iterates sources in configurable order (`source_order`),
trying each until one returns successfully. The per-source `_enabled` booleans
act as a filter — disabled sources are skipped entirely.

```
request → [GitHub Releases] ?→ [Baller Registry] ?→ [Chocolatey Feed]
              ↓ failure           ↓ failure               ↓ failure
          try next             try next              return error
```

## Dependency Resolution

`resolve_deps()` in `dep_solver.rs`:
1. BFS traversal from the root package
2. Fetches metadata for each dependency from the registry
3. Builds a dependency graph
4. Validates version constraints (using `semver::VersionReq`)
5. Detects cycles (DFS with White/Gray/Black coloring)
6. Topological sort (DFS post-order) for install order
7. Returns packages in dependency-first order
