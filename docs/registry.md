# Registry System

Baller supports multiple package sources with a configurable fallback chain.
The `RegistryClient` in `src/core/registry.rs` coordinates all sources.

## Supported Sources

### 1. GitHub Releases (`src/http/github.rs`)

Queries the GitHub Releases API at `api.github.com/repos/{owner}/{repo}/releases/latest`.

- Matches assets by platform (Linux/Windows) and architecture (x86_64/aarch64)
- Parses owner/repo from the package source metadata
- Falls back to search via repository topic tags

### 2. Baller Registry API (`src/http/registry_api.rs`)

A custom registry API at a configurable base URL (default `https://registry.baller.dev/api`).

- Versioned package metadata endpoint
- Search endpoint for scouting packages
- Structured JSON responses with dependency information

### 3. Chocolatey/NuGet Feed (`src/http/chocolatey.rs`)

Queries a NuGet OData v2 feed (default `https://community.chocolatey.org/api/v2`).

- Uses OData protocol for package lookup
- Normalizes NuGet version schemes to semver
- Parses dependency strings from NuGet package metadata

### 4. System Package Manager (`src/http/system.rs`) — NEW

Wraps the system's native Linux package manager (apt, dnf, or pacman) via CLI commands.
Only active on Linux systems; auto-detects distro from `/etc/os-release`.

- **Distro detection**: Reads `ID` field from `/etc/os-release`, maps to apt/dnf/pacman
- **Detection table**:
  - Ubuntu, Debian, Linux Mint, Pop!_OS, Elementary OS, Zorin, Kali, Raspbian → `apt`
  - Fedora, RHEL, CentOS, Rocky Linux, AlmaLinux, Oracle Linux, Nobara → `dnf`
  - Arch Linux, Manjaro, EndeavourOS, Garuda Linux, Arco Linux → `pacman`
- **Fetch**: Shells to `apt-cache show`, `dnf info`, or `pacman -Si` for package metadata
- **Search**: Shells to `apt-cache search`, `dnf search`, or `pacman -Ss` for package listings
- **Dependencies**: Parses native dependency lists (comma-separated for apt, space-separated for dnf/pacman)
- **No HTTP client needed** — operates entirely via local CLI

```rust
// Package source metadata includes the detected manager
PackageSource::System { manager: "apt" }  // or "dnf" or "pacman"
RegistrySource::System                     // registry-level source enum
```

## Fallback Chain

Sources are tried in the order specified by `source_order` in the config.
The first source to return a successful result wins.

```
request → [GitHub] ?→ [Baller Registry] ?→ [Chocolatey]
              ↓ failure         ↓ failure            ↓ failure
          try next           try next           return error
```

## Configuration

```ini
[registry]
# Source resolution order (comma-separated)
source_order = github,baller,chocolatey

# Custom registry URLs
baller_registry_url = https://registry.baller.dev/api
chocolatey_feed_url = https://community.chocolatey.org/api/v2

# Per-source enable/disable flags
github_enabled = true
baller_enabled = true
chocolatey_enabled = true
system_enabled = true
```

### Per-Source Enable/Disable

Each source has an `_enabled` boolean flag. When disabled, the source is skipped
entirely during the fallback chain — useful for offline work or isolating issues.

Disabling a source takes precedence over `source_order` — a disabled source is
silently dropped from the resolved list.

```ini
# Use only the baller registry, skip GitHub and Chocolatey
source_order = github,baller,chocolatey,system
github_enabled = false
chocolatey_enabled = false
```

## RegistryClient API

```rust
// Fetch the latest version of a package (tries sources in order)
let pkg = client.fetch_package("ripgrep")?;

// Search across all sources
let results = client.search("rip")?;

// Fetch from a specific source
let pkg = client.fetch_package_from_source(&RegistrySource::GitHub, "ripgrep")?;
```

## Adding a Custom Source

To add a new registry source:

1. Create a new module in `src/http/` (e.g., `src/http/my_registry.rs`)
2. Implement the `RegistryIndex` trait (`fetch_package` + `search`)
3. Add a variant to the `PackageSource` enum in `src/core/package.rs`
4. Add a variant to the `RegistrySource` enum in `src/core/registry.rs`
5. Wire it into `RegistryClient` in `src/core/registry.rs`
6. Add config keys for the new source in `src/config/config.rs`
7. Add the enabled flag filter in `AppContext::new()` in `src/context.rs`

### Local CLI Source Pattern (System Registry)

The System registry (`src/http/system.rs`) demonstrates a non-HTTP registry source.
Key differences from HTTP-based sources:

- No `HttpClient` dependency — use `std::process::Command` for CLI calls
- Auto-detects capability at construction time via `detect()` (returns `None` if no PM found)
- Gracefully returns `BallError::PackageManagerError` when no manager is available
- Stores detected manager in `PackageSource::System { manager: "apt" }` for provenance

## System Registry Details

See [docs/system-registry.md](system-registry.md) for the complete reference.
