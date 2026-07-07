# Baller Implementation Plan

## Overview

**Baller** — The Binary Allocation & Library Launch Environment in Rust. A cross-platform package manager (Windows/Linux) supporting multiple package sources with a sports-themed CLI.

---

## Legend

- ✅ **Completed (Phases 0–5)**
- 🔜 **Remaining (Phases 6–9)**

---

## Phase 0 — Consolidation & Setup ✅

**Goal**: Merge the two divergent codebases into one working project.

- **Base**: Use `Documents/BALLER/baller` as the primary source (sports-themed naming, `clap` derive CLI, `core/` and `platform/` modules)
- **Cargo.toml**: Full dependencies section — `clap`, `colored`, `indicatif`, `reqwest`, `rusqlite`, `serde`, `serde_json`, `sha2`, `thiserror`, `toml`
- **Added deps**: `zip`, `flate2`, `tar`, `semver`, `url`
- **Verify**: `cargo build` compiles cleanly

---

## Phase 1 — Registry System (`src/http/` + `src/core/registry.rs`) ✅

**Goal**: Fetch package metadata and download URLs from multiple sources.

### `src/http/mod.rs`
- Reqwest HTTP client wrapper with timeouts, retries, and user-agent header

### `src/http/github.rs`
- GitHub Releases API client
- Queries `api.github.com/repos/{owner}/{repo}/releases/latest`

### `src/http/registry_api.rs`
- Custom baller registry API client
- Community-run index at a configurable URL

### `src/http/chocolatey.rs`
- Chocolatey/NuGet OData feed client
- Queries `chocolatey.org/api/v2` / NuGet protocol

### `src/core/registry.rs`
- Unified `RegistryIndex` trait with methods:
  - `fetch_package(name, version)` → `Package` metadata
  - `search(query)` → `Vec<Package>`
  - `resolve_latest(name)` → version string
- `PackageSource` enum: `GitHub`, `BallerRegistry`, `Chocolatey`
- Fallback chain: try sources in configured order until one succeeds
- Config: `[registry]` section in `baller.conf` — `sources_order`, `github_fallback`, custom registry URL

---

## Phase 2 — Database & State (`src/core/db.rs`) ✅

**Goal**: Track all installed packages, versions, pins, and dependencies.

### Schema

```sql
CREATE TABLE IF NOT EXISTS packages (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    version TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'github',
    frozen BOOLEAN NOT NULL DEFAULT 0,
    install_path TEXT,
    bin_path TEXT,
    manifest_path TEXT,
    installed_at TEXT NOT NULL DEFAULT (datetime('now')),
    sha256 TEXT
);

CREATE TABLE IF NOT EXISTS dependencies (
    pkg_id INTEGER NOT NULL,
    dep_name TEXT NOT NULL,
    dep_version TEXT NOT NULL,
    FOREIGN KEY (pkg_id) REFERENCES packages(id)
);

CREATE TABLE IF NOT EXISTS lockfile (
    name TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    source TEXT NOT NULL,
    sha256 TEXT
);
```

### Methods
- `init()` — Open/create database, initialize schema
- `insert_package(pkg, bin_path)` — Insert or update (upsert on name)
- `remove_package(name)` — Delete from DB
- `get_package(name)` → `Package`
- `list_packages()` → `Vec<Package>`
- `search_installed(query)` → `Vec<Package>`
- `is_frozen(name)` / `set_frozen(name, bool)` — Pin/unpin
- `get_all_frozen()` → `Vec<String>`
- `lockfile_sync()` — Read/write lock file

---

## Phase 3 — Package Download & Verification (`src/core/downloader.rs`) ✅

**Goal**: Download, verify, and extract packages.

- Download with `reqwest` and progress bars via `indicatif`
- Support archive formats: `.tar.gz`, `.zip`
- SHA-256 hash verification after download using `sha2`
- Cache downloaded archives to `~/.baller/cache/`
- Extract binaries to a staging directory
- Clean up staging after successful installation

---

## Phase 4 — Command Implementations (`src/commands/`) ✅

**Goal**: Fully implement all CLI commands.

### `draft` (Install)
1. Resolve package name → fetch metadata from registry chain
2. Resolve dependencies recursively via `dep_solver`
3. Check if already installed → skip or prompt reinstall
4. Download archive with progress bar
5. Verify SHA-256 hash
6. Extract binary to staging
7. Platform: `create_symlink()` into install dir (`~/.local/bin` on Linux, `%LOCALAPPDATA%/baller/bin` on Windows)
8. Insert into SQLite `packages` table
9. Run post-install hooks
10. Clean up staging

### `eject` (Uninstall)
1. Check if package exists in DB
2. Check if frozen → error if so
3. Run pre-eject hooks
4. Platform: `remove_symlink()`
5. Remove binary from install dir
6. Remove from SQLite
7. Optionally remove unused dependencies (with `--orphans` flag)

### `roster` (List/Search)
- **No args**: List all installed packages with versions (from DB)
- **`-r` flag**: Search remote registries for packages
- **`-d` flag**: Show dependency tree
- **`<name>`**: Show detailed info for specific package
- Colored output with `colored` crate

### `update` (Update Packages)
- **No args**: Update all non-frozen packages
- **`<name>`**: Update specific package
- Check frozen status before updating
- Pipeline: fetch latest → download → hash verify → backup old → install new → update DB
- Skip if already at latest

### `freeze` / Unfreeze
- `freeze <name>`: Set `frozen=1` in DB → prevents update/eject
- `unfreeze <name>`: Set `frozen=0`
- `freeze list`: Show all frozen packages

### `substitute` (Swap Packages)
1. Fetch new package metadata
2. Check if old is installed
3. Resolve new's dependencies
4. Install new package
5. Remove old package binary and symlink
6. Update DB (remove old, insert new)
7. Re-link any dependents that pointed to old

### `sweep` (Cache Cleanup)
- Remove `~/.baller/cache/` contents (downloaded archives)
- Remove orphaned staging directories
- Remove stale lock files
- Optional `--dry-run` flag to preview
- Optional `--all` to also purge DB history

### `build` (From Manifest)
1. Parse manifest (`baller.toml` or `baller.json`)
2. Resolve and download dependencies
3. If source URL present, download source archive
4. Build or place binary
5. Register built package in DB
6. Create symlink

---

## Phase 5 — Config Expansion (`src/config/config.rs`) ✅

**Goal**: Support all configuration options in a structured format.

```toml
[baller]
install_dir = "~/.local/bin"
cache_dir = "~/.baller/cache"
db_path = "~/.baller/db/baller.db"

[registry]
sources_order = ["github", "baller", "chocolatey"]
github_fallback = true
baller_registry_url = "https://registry.baller.dev/api"
chocolatey_enabled = true

[hooks]
pre_install = true
post_install = true
pre_eject = true
```

---

## Phase 6 — Hook System (`src/core/hooks.rs`) 🔜

**Goal**: Run user-defined scripts before/after package operations.

- Hook types: `PreInstall`, `PostInstall`, `PreEject`, `PostEject`, `PreUpdate`, `PostUpdate`
- Hook scripts stored in `~/.baller/hooks/{name}_{type}.{sh,ps1}`
- `HookRunner` executes scripts and captures output
- Linux: `bash -c`; Windows: `powershell -Command`
- Hooks can abort operations by returning non-zero exit code
- Respect `[hooks]` config section (enable/disable per hook type)

---

## Phase 7 — Dependency Resolution (`src/core/dep_solver.rs`) 🔜

**Goal**: Properly resolve and install transitive dependencies.

- Parse version constraints: `>=1.0`, `^2.3`, `~1.2.3`, `*`
- Use `semver` crate for version comparison
- Build dependency graph from registry metadata
- Detect cycles and report errors with package names
- BFS topological sort to determine install order
- Handle conflicts (two packages requiring incompatible versions of same dep)
- Respect frozen packages in the graph
- Support optional dependencies

---

## Phase 8 — Documentation (`docs/`) 🔜

**Goal**: Populate the empty `docs/` directory.

- `docs/architecture.md` — High-level component overview
- `docs/commands.md` — Full CLI reference with examples for all 8 commands
- `docs/manifest.md` — `baller.toml` / `baller.json` format specification
- `docs/hooks.md` — Hook system reference (script locations, environment variables)
- `docs/registry.md` — How the multi-source registry works, how to add custom sources
- `docs/contributing.md` — Development guide, how to build and test

---

## Phase 9 — Testing & Polish 🔜

**Goal**: Ensure correctness, reliability, and good user experience.

### Unit Tests
- Each command module: draft, eject, roster, update, freeze, substitute, sweep, build
- Registry client: mock HTTP responses for each source
- Dependency solver: various version constraint scenarios
- Database: CRUD operations, schema migration
- Config parser: valid configs, edge cases, error handling

### Integration Tests
- End-to-end: mock registry → draft → verify installed → roster → update → eject
- Platform symlink creation/removal (Linux: symlink, Windows: copy)
- Hook execution (script runs, abort on error)

### CLI Polish
- `--help` with per-command flag descriptions
- Colored output for errors (red) vs success (green) using `colored` crate
- Progress bars on downloads using `indicatif`
- Tab completion scripts (bash, zsh, powershell)

### Code Quality
- Remove all `TODO` comments and stubs
- Fix all compiler warnings (`unused`, `dead_code`)
- Run `cargo clippy` and address all lint warnings
- Run `cargo fmt`

---

## Final Directory Structure

```
baller/
├── Cargo.toml
├── Cargo.lock
├── IMPLEMENTATION_PLAN.md
├── docs/
│   ├── architecture.md
│   ├── commands.md
│   ├── manifest.md
│   ├── hooks.md
│   ├── registry.md
│   └── contributing.md
├── src/
│   ├── main.rs
│   ├── cli/
│   │   ├── mod.rs
│   │   ├── parse.rs          # clap derive
│   │   ├── help.rs
│   │   └── version.rs
│   ├── commands/
│   │   ├── mod.rs
│   │   ├── draft.rs          # install
│   │   ├── eject.rs          # uninstall
│   │   ├── roster.rs         # list/search
│   │   ├── sweep.rs          # clean cache
│   │   ├── freeze.rs         # pin packages
│   │   ├── substitute.rs     # swap packages
│   │   ├── build.rs          # build from manifest
│   │   └── update.rs         # update packages
│   ├── config/
│   │   ├── mod.rs
│   │   └── config.rs
│   ├── core/
│   │   ├── mod.rs
│   │   ├── db.rs             # SQLite state
│   │   ├── dep_solver.rs     # dependency resolution
│   │   ├── hooks.rs          # pre/post scripts
│   │   ├── manifest.rs       # parse baller.toml/json
│   │   ├── package.rs        # Package struct
│   │   ├── registry.rs       # multi-source registry client
│   │   └── downloader.rs     # download, verify, extract
│   ├── error/
│   │   ├── mod.rs
│   │   └── error.rs
│   ├── http/
│   │   ├── mod.rs
│   │   ├── github.rs         # GitHub Releases client
│   │   ├── registry_api.rs   # baller registry API client
│   │   └── chocolatey.rs     # Chocolatey/NuGet client
│   ├── platform/
│   │   ├── mod.rs
│   │   ├── common.rs         # PlatformManager trait
│   │   ├── linux.rs          # Linux symlink impl
│   │   └── windows.rs        # Windows copy/symlink impl
│   └── utils/
│       ├── mod.rs
│       ├── fs.rs             # file helpers
│       └── security.rs       # hash verification
```
