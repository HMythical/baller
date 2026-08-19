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

- **Accept header** must be `application/json;odata=verbose` — the OData v2
  endpoint rejects plain `application/json` with HTTP 406 Not Acceptable
- Uses OData protocol for package lookup
- Normalizes NuGet version schemes to semver (strips trailing `.0` build revisions)
- Parses dependency strings from NuGet package metadata
- **Download URL** is derived from `__metadata.media_src` in the OData response
  (the `DownloadUrl` property does not exist on the v2 feed and requests that
  include it in `$select` are rejected). When `media_src` is absent, the URL
  falls back to `https://community.chocolatey.org/api/package/{id}/{version}`.
- **Hash verification** uses the algorithm reported by Chocolatey
  (`PackageHashAlgorithm` field). The hash is base64-encoded — it is decoded
  and compared against the SHA-512 (or SHA-256) hex digest of the downloaded
  archive. The `hash_algorithm` field on the `Package` struct carries the
  algorithm name so the downloader selects the correct verification path.

### 4. System Package Manager (`src/http/system.rs`)

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
- **Install**: Runs `sudo <pm> install -y <name>` to install packages via the native manager
- **No HTTP client needed** — operates entirely via local CLI

```rust
// Package source metadata includes the detected manager
PackageSource::System { manager: "apt" }  // or "dnf" or "pacman"
RegistrySource::System                     // registry-level source enum
```

## Fallback Chain

Sources are tried in the order specified by `source_order` in the config.
The first source to return a successful result wins. When **all** sources
fail, the error message lists every source that was tried and the reason
it failed (so you can see why GitHub returned "not found" *and* why
Chocolatey returned 406, not just the last one).

```
Windows: request → [Baller Registry] ?→ [Chocolatey] ?→ [GitHub]
Linux:   request → [Baller Registry] ?→ [System PM]  ?→ [GitHub]
                          ↓ failure          ↓ failure       ↓ failure
                          try next           try next        return error
```

The default `source_order` is platform-derived:

| Platform | Default `source_order` |
|---|---|
| Windows | `baller, chocolatey, github` |
| Linux | `baller, system, github` |

The Baller registry comes first (it is not implemented yet, so it currently
fails fast and falls through), the platform's native ecosystem comes next, and
**GitHub is always the last-resort fallback** so GitHub-based installs keep
working everywhere.

A source that does not belong to the platform is disabled by default —
`chocolatey_enabled` is `false` on Linux and `system_enabled` is `false` on
Windows — and `SystemRegistry` detection is skipped entirely off Linux. Both
remain available as an explicit opt-in via the `_enabled` flags below.

Entries in `source_order` that match no known source are ignored.

## Configuration

```ini
[registry]
# Source resolution order (comma-separated), overrides the platform default
source_order = baller,chocolatey,github

# Custom registry URLs
baller_registry_url = https://registry.baller.dev/api
chocolatey_feed_url = https://community.chocolatey.org/api/v2

# Per-source enable/disable flags (defaults follow the platform)
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
source_order = baller,chocolatey,github
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
