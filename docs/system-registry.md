# System Registry (Local Package Manager)

## Overview

The `SystemRegistry` in `src/http/system.rs` wraps the system's native Linux package manager
through CLI commands, enabling BALLER to query and install packages from the OS-level repositories.

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
- `version` → Package.version
- `description`/`summary` → Package.description
- `maintainer`/`packager` → Package.author
- `depends` → Package.dependencies (deduplicated)
- `download_url` = `None` (system PM handles downloads)
- `sha256` = `None`

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
