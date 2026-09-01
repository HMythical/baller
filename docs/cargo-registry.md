# Cargo Registry (crates.io via the local toolchain)

## Overview

The `CargoRegistry` in `src/http/cargo.rs` wraps the locally installed cargo toolchain
through CLI commands, enabling BALLER to query and install crates from crates.io.

Cargo packages are **not** downloaded or extracted by BALLER — cargo compiles and
installs them in place into `~/.cargo/bin`. The downloader refuses to handle a
`PackageSource::Cargo` package, and `draft`/`build` dispatch cargo packages to
`install_cargo_package()` which shells out to `cargo install`. Unlike the system
package manager, cargo installs are user-local and never run under `sudo`.

## Architecture

```
CargoRegistry
├── available: bool                 // Probed at construction (false = no cargo on PATH)
│
├── detect()          → CargoRegistry  (probes `cargo --version`)
├── manager_name()    → Option<&str>   ("cargo" when available, None otherwise)
├── fetch_package()   → Result<Package, BallError>  (crate metadata)
└── search()          → Result<Vec<Package>, BallError>  (crate listings)
```

## Toolchain Detection

Detection runs `cargo --version` and records whether it exited successfully. A host
with no cargo on `PATH` degrades exactly like `SystemRegistry` on a host with no
package manager: `manager_name()` returns `None` and every fetch/search returns
`BallError::PackageManagerError`.

Cargo is available on every platform, so — unlike the system source — the Cargo
source is not gated on `cfg!(target_os = "linux")`. It is only *enabled by default*
on Linux (see [Configuration](#configuration)).

## CLI Commands

### Package Fetch (`fetch_package`)

| Step | Command | Key Fields Parsed |
|------|---------|-------------------|
| **Primary** | `cargo info <name>` | name, description, `version:`, `repository:` |
| **Fallback** | `cargo search <name> --limit 20` | exact-name match from the search listing |

`cargo info` was added in a later cargo release; when it is unavailable (or exits
non-zero) the fallback parses `cargo search` output and takes the entry whose name
matches exactly. If neither yields a match, `BallError::PackageNotFound(name)` is
returned.

Fields filled:
- `version` → Package.version
- description block → Package.description
- `repository:` → Package.repository
- `author` = `None` (`cargo info` does not report crate owners)
- `dependencies` = `None` (`cargo info` reports features, not dependencies)
- `download_url` = `None` (cargo handles the download)
- `sha256` = `None` (cargo verifies crates against the registry index)

### Version Behavior

crates.io enforces valid semver, so no normalization is needed — versions parse
directly through `parse_version_flexible()` in `src/core/dep_solver.rs`.

`cargo info` prints `version: 1.0.228 (latest 1.0.229)` when a lockfile in the
current directory pins an older release than crates.io carries. The parser takes
the **latest** value in that case, because that is the version `cargo install`
resolves to.

**Version pinning is not supported.** Like the system source, Cargo always installs
the latest available release; `--version` against the Cargo source returns:

```
cargo packages always install the latest available version — cannot pin '<name>'
```

### Package Search (`search`)

| Command | Output Format |
|---------|---------------|
| `cargo search <query> --limit 20` | `name = "version"    # description` per line |

Lines that do not carry a quoted version — cargo's `... and N crates more` total and
its trailing `note:` hint — are skipped. Search results are lightweight: name,
version, description, and source only.

## PackageSource Serialization to DB

Cargo packages serialize to the database as:

```rust
("cargo".to_string(), Some(crate_name.clone()))  // e.g., ("cargo", Some("ripgrep"))
```

This preserves provenance — queries can filter by `source="cargo"` and
`source_detail="<crate>"`.

## Manifests

A manifest selects the Cargo source with `type = "cargo"` (`"crate"` is accepted as
an alias). The crate name comes from `crate_name`, falling back to the source's
`name` field and then to the package name:

```toml
name = "ripgrep"
version = "14.1.1"

[source]
type = "cargo"
crate_name = "ripgrep"
```

## Error Handling

| Scenario | Error Returned |
|----------|---------------|
| No cargo toolchain detected | `BallError::PackageManagerError("cargo is not installed on this host")` |
| Crate not found (no info/search match) | `BallError::PackageNotFound(name)` |
| Command execution failure | `BallError::PackageManagerError(format!("failed to run {}: {}", cmd, e))` |
| Non-zero exit from a query | `BallError::PackageManagerError(format!("{} exited with status {}", cmd, status))` |
| Installation failure (non-zero exit) | `BallError::PackageManagerError(format!("cargo install of '{}' exited with status {}", crate_name, status))` |
| `--source cargo` with no cargo on PATH | `BallError::PackageManagerError("--source cargo needs a cargo toolchain on PATH")` |
| Version pin requested against Cargo | `BallError::PackageManagerError("cargo packages always install the latest available version — cannot pin '<name>'")` |
| Wrong code path tries to download a cargo package | `BallError::PackageManagerError(format!("'{}' is a cargo package — use cargo install", name))` |

## Configuration

```ini
[registry]
source_order = baller,system,cargo,github    # cargo must be in list
cargo_enabled = true                          # toggle on/off
```

`cargo_enabled` defaults to `true` on Linux and `false` on Windows, where Chocolatey
is the native ecosystem. It remains an explicit opt-in everywhere else.

When `cargo_enabled = false`:
- `"cargo"` is filtered out of the effective source order
- `CargoRegistry` still exists but won't be queried during fetch/search
- `--source cargo` still resolves from it explicitly

## Testing

Full test coverage in `src/http/cargo.rs#tests`:

| Test | What it verifies |
|------|-----------------|
| `test_parse_cargo_search_output` | `cargo search` parsing (name, version, description; description-less lines) |
| `test_parse_cargo_search_output_stamps_cargo_source` | Search results carry `PackageSource::Cargo` and no download URL/hash |
| `test_parse_cargo_search_output_skips_notes_and_totals` | `... and N crates more` and `note:` lines are ignored |
| `test_parse_cargo_info` | `cargo info` parsing (name, version, description, repository, source) |
| `test_parse_info_version_without_latest` | Plain versions pass through; `(latest X)` wins when present |
| `test_parse_cargo_info_keeps_description_containing_colon` | A colon in the description is not mistaken for a field |
| `test_parse_cargo_info_empty_output_is_not_found` | Empty output → `PackageNotFound` |
| `test_parse_cargo_info_without_version_is_not_found` | Output with no `version:` → `PackageNotFound` |
| `test_available_registry_reports_manager` | Probed-available registry reports `Some("cargo")` |
| `test_unavailable_registry_has_no_manager` | Probed-unavailable registry reports `None` |
| `test_unavailable_registry_errors_on_fetch_and_search` | No-cargo fetch/search return `PackageManagerError` |

Tests parse fixture strings inline and never invoke the real `cargo` binary.
