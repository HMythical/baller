# System Registry (Local Package Manager)

## Overview

The `SystemRegistry` in `src/http/system.rs` wraps the system's native Linux package manager
through CLI commands, enabling BALLER to query and install packages from the OS-level repositories.

System packages are **not** downloaded or extracted by BALLER — they are installed
in place by the native package manager (`apt-get install`, `dnf install`,
`pacman -S`). The downloader refuses to handle a `PackageSource::System`
package and `draft` dispatches system packages to `install_system_package()`
which shells out to the appropriate CLI under `sudo`.

## Architecture

```
SystemRegistry
├── manager: Option<SystemManager>   // Auto-detected at construction (None = no PM)
│   ├── Apt       → apt-cache / dpkg
│   ├── Dnf       → dnf
│   └── Pacman    → pacman
│
├── detect()          → SystemRegistry  (auto-detect distro)
├── fetch_package()   → Result<Package, BallError>  (package metadata)
└── search()          → Result<Vec<Package>, BallError>  (package listings)
```

## Distro Detection

Detection reads `/etc/os-release` and matches the `ID=` field:

| Distros | Package Manager |
|---------|----------------|
| ubuntu, debian, linuxmint, pop, elementary, zorin, kali, raspbian | apt |
| fedora, rhel, centos, rocky, almalinux, ol, nobara | dnf |
| arch, manjaro, endeavouros, garuda, arco | pacman |

Unknown or missing `os-release` → `None` (graceful no-op).

## CLI Commands

### Package Fetch (`fetch_package`)

| Manager | Command | Key Fields Parsed |
|---------|---------|-------------------|
| **apt** | `apt-cache show <name>` | Package, Version, Description, Maintainer, Depends |
| **dnf** | `dnf info <name>` | Name, Version, Summary, Packager |
| **pacman** | `pacman -Si <name>` | Name, Version, Description, Depends |

Fields filled:
- `version` → Package.version (may use Debian epoch `2:1.21-76` or RPM release `8.2.2637-20.fc36` formats)
- `description`/`summary` → Package.description
- `maintainer`/`packager` → Package.author
- `depends` → Package.dependencies (deduplicated; parenthesized version constraints like `(= 2:1.21-76)` are stripped, leaving just the package name)
- `download_url` = `None` (system PM handles downloads)
- `sha256` = `None`

### Version Format Compatibility

System package versions are **not** guaranteed to be valid semver. Common
non-semver formats include:

| Format | Example | Notes |
|--------|---------|-------|
| Debian epoch | `2:1.21-76` | Epoch prefix `2:` is dropped before semver comparison |
| Debian revision | `1.21-76` | Revision suffix `-76` is dropped before semver comparison |
| Debian embedded tags | `1:1.3.dfsg+really1.3.1-1+b1` | Epoch, revision, and embedded tags (`dfsg+really`) are stripped; only leading numeric segments are kept (`1.3`) |
| RPM release | `8.2.2637-20.fc36` | Release tag is dropped before semver comparison |
| Date-based versions | `20240118` | Padded to `20240118.0.0` for semver compatibility |
| Multi-segment | `1.4.309.0-1` | Trimmed to 3 segments (`1.4.309`) for semver compatibility |
| 2-part versions | `1.21` | Padded with `.0` (`1.21.0`) |
| Strict semver | `1.2.3` | Parsed directly |

The dependency resolver in `src/core/dep_solver.rs` uses
`parse_version_flexible()` which strips the epoch, revision, embedded tags,
and build metadata before parsing. All versions are normalized to exactly
3 semver segments (major.minor.patch). If the cleaned value still cannot be
parsed as semver, a `BallError::PackageManagerError` is returned.

### Dependency Resolution Behavior

System packages often have characteristics that differ from typical
GitHub/Chocolatey packages:

- **Virtual packages**: Debian virtual packages (e.g., `default-dbus-session-bus`)
  are dependency targets that don't exist as standalone installable packages.
  `resolve_deps()` skips `PackageNotFound` errors for individual dependencies
  rather than failing the entire resolution.

- **Dependency cycles**: System packages commonly have mutual dependencies
  (e.g., `libc6 ↔ libgcc-s1`). The resolver skips cycle detection for
  system packages and returns the best available dependency ordering.

- **Unparseable versions**: When a dependency's version cannot be normalized
  to semver by `parse_version_flexible()`, the resolver returns a
  `PackageManagerError` with the problematic version string.

### Package Search (`search`)

| Manager | Command | Output Format |
|---------|---------|---------------|
| **apt** | `apt-cache search <query>` | `name - description` per line |
| **dnf** | `dnf search <query>` | Name/Summary pairs separated by lines |
| **pacman** | `pacman -Ss <query>` | `repo/name version` then indented description |

Search results are lightweight: only name, description, and source. Version and other fields are empty.

## PackageSource Serialization to DB

System packages serialize to the database as:

```rust
("system".to_string(), Some(manager.clone()))  // e.g., ("system", Some("apt"))
```

This preserves provenance — queries can filter by `source="system"` and `source_detail="apt"`.

## Error Handling

| Scenario | Error Returned |
|----------|---------------|
| No package manager detected | `BallError::PackageManagerError("no system package manager detected")` |
| Package not found (non-zero exit code) | `BallError::PackageNotFound(name)` |
| Command execution failure | `BallError::PackageManagerError(format!("failed to run {}: {}", cmd, e))` |
| Installation failure (non-zero exit) | `BallError::PackageManagerError(format!("{} install of '{}' exited with status {}", manager, name, status))` |
| Unsupported manager (e.g. `apk`) | `BallError::PackageManagerError(format!("unsupported system package manager: {}", manager))` |
| Wrong code path tries to download a system package | `BallError::PackageManagerError(format!("'{}' is a system package — use native package manager", name))` |

## Configuration

```ini
[registry]
source_order = github,baller,chocolatey,system    # system must be in list
system_enabled = true                              # toggle on/off
```

When `system_enabled = false`:
- `"system"` is filtered out of the effective source order
- `SystemRegistry` still exists but won't be queried during fetch/search

## Testing

Full test coverage in `src/http/system.rs#tests`:

| Test | What it verifies |
|------|-----------------|
| `test_detect_system_manager_debian` | Debian → Apt mapping with temp os-release file |
| `test_detect_system_manager_fedora` | Fedora → Dnf mapping |
| `test_detect_system_manager_arch` | Arch → Pacman mapping |
| `test_detect_system_manager_manjaro` | Manjaro → Pacman mapping (variant) |
| `test_detect_system_manager_unknown` | Unknown distro → None |
| `test_detect_missing_file_returns_none` | Missing os-release → None |
| `test_detect_all_apt_ids` | All 8 Apt-family distros |
| `test_detect_all_dnf_ids` | All 7 Dnf-family distros |
| `test_detect_all_pacman_ids` | All 5 Pacman-family distros |
| `test_parse_apt_cache_show_output` | apt-cache show parsing (version, description, author, deps) |
| `test_parse_apt_deps_dedup` | Dependency deduplication |
| `test_parse_dnf_info_output` | dnf info parsing |
| `test_parse_pacman_si_output` | pacman -Si parsing |
| `test_parse_apt_cache_search_output` | apt-cache search parsing |
| `test_parse_pacman_ss_output` | pacman -Ss parsing |
| `test_search_returns_lightweight_packages` | Search results have empty version, populated name/description |
| `test_system_registry_no_pm_fetch_error` | No-PM fetch returns PackageManagerError |
| `test_system_registry_no_pm_search_error` | No-PM search returns PackageManagerError |
