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
│   │   ├── chocolatey.rs    # Chocolatey OData v2 feed
│   │   └── system.rs        # System Linux package manager (apt/dnf/pacman)
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
- `source` (`PackageSource` enum: `GitHub`, `BallerRegistry`, `Chocolatey`, `System { manager }`)
- `download_url`, `sha256`, `hash_algorithm`, `dependencies`

The `hash_algorithm` field (e.g., `"SHA256"`, `"SHA512"`) is set by the
Chocolatey source to indicate which hash algorithm was used. When present,
the downloader decodes the base64 hash and verifies using the specified
algorithm. For GitHub/Baller sources, `hash_algorithm` is `None` and the
downloader defaults to SHA-256 hex verification.

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

The default `source_order` is `github, baller, chocolatey, system`, so
system packages (apt/dnf/pacman) are tried as a last resort. When all
sources fail, the returned error lists every source that was tried and the
reason it failed.

```
request → [GitHub Releases] ?→ [Baller Registry] ?→ [Chocolatey Feed] ?→ [System PM]
              ↓ failure           ↓ failure               ↓ failure            ↓ failure
          try next             try next              try next              return error
```

## Dependency Resolution

`resolve_deps()` in `dep_solver.rs`:
1. BFS traversal from the root package
2. Fetches metadata for each dependency from the registry
3. Builds a dependency graph
4. Validates version constraints (using `semver::VersionReq`)
5. Detects cycles (DFS with White/Gray/Black coloring) — skipped for system packages
6. Topological sort (DFS post-order) for install order
7. Returns packages in dependency-first order

**System package resilience**: The resolver tolerates common characteristics
of system packages:
- **Virtual packages** (e.g., `default-dbus-session-bus`): `PackageNotFound`
  errors for individual dependencies are skipped rather than failing the
  entire resolution.
- **Dependency cycles** (e.g., `libc6 ↔ libgcc-s1`): Cycle detection is
  skipped and the topological sort returns the best available ordering.
- **Unparseable versions**: If `parse_version_flexible()` cannot normalize
  a version to semver, a `PackageManagerError` is returned.

### Version Format Flexibility

Versions from the system package manager (e.g. `2:1.21-76`, `8.2.2637-20.fc36`)
are **not** strict semver. `parse_version_flexible()` in `dep_solver.rs` handles:

1. **Epoch prefix stripping** (`2:1.21` → `1.21`)
2. **Revision suffix stripping** (`1.21-76` → `1.21`, `8.2.2637-20.fc36` → `8.2.2637`)
3. **Embedded tag stripping** (`1.3.dfsg+really1.3.1` → `1.3`)
4. **Build metadata stripping** (`1.0+build` → `1.0`)
5. **Normalization to 3 segments** (`1.4.309.0` → `1.4.309`, `20240118` → `20240118.0.0`, `1.21` → `1.21.0`)
6. **Pre-release handling**: Versions with semver pre-release tags (e.g., `1.21.76-2`) are
   treated as revision-stripped (`-2` is a Debian revision, not a semver pre-release)

## Installation Paths

BALLER supports three installation paths, selected automatically based on
`PackageSource`:

- **Archive path** (GitHub / BallerRegistry): download archive →
  SHA-256 hex verify → extract → symlink binary → record in DB.
- **Chocolatey path** (Chocolatey/NuGet): download `.nupkg` archive →
  SHA-512 base64 decode and verify → extract as zip → record in DB.
  NuGet `.nupkg` files are zip archives and are extracted with the zip handler.
  Archives without a recognized extension (e.g., cached from API URLs) fall
  back to zip extraction.
- **System path** (`PackageSource::System`): delegate to native package
  manager via `install_system_package()` in `http/system.rs`, which runs
  `sudo apt-get install -y <name>` (or dnf/pacman equivalent) and records
  the package in the DB. No archive is downloaded, no symlink is created.

The downloader (`Downloader::download_and_extract`) refuses to handle
system packages, returning a `PackageManagerError` if invoked on one — this
is a defensive check, since the `draft` command should always dispatch
system packages to the system path.

## Error Handling and Rollback

### Partial Failure Rollback
Commands that install multiple packages (`draft`, `substitute`) track packages
installed during the current session. If any package fails to install, all
packages from the session are rolled back (symlink removed, DB entry deleted).

### Orphan Dependency Cleanup
When ejecting a package, BALLER checks for orphaned dependencies — packages
that were installed as transitive dependencies and are no longer required by
any remaining installed package. Orphans are automatically removed with a
warning message. This relies on the `user_installed` flag in the database:
packages installed as dependencies are marked `user_installed = false`, while
packages explicitly installed via `draft` are marked `user_installed = true`.

### Confirmation Prompts
Destructive commands (`eject`, `sweep`) prompt for confirmation unless the
`--yes` / `-y` flag is passed.

### Version Comparison
The `update` command uses `parse_version_flexible()` for version comparison,
which handles Debian epoch prefixes, revision suffixes, embedded tags,
date-based versions, and multi-segment versions — not just strict semver.
Falls back to string comparison when both versions cannot be parsed.
