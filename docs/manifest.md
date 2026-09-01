# Manifest Format

Baller uses `baller.toml` (preferred) or `baller.json` manifest files to describe
packages. The manifest is parsed by the `ManifestParser` in `src/core/manifest.rs`
and consumed by [`baller build`](commands.md).

**Two layouts parse:** the *nested* layout below (grouped `[source]`,
`[checksum]`, `[architectures]`, `[dependencies]` tables) and the *flat* layout
that mirrors the internal `Package` struct. The parser normalizes the nested
layout into the flat model, so the two are interchangeable. Serialization always
emits the flat layout.

## TOML Format (nested)

```toml
name = "ripgrep"
version = "14.1.0"
description = "Blazingly fast search tool"
author = "Andrew Gallant"
repository = "https://github.com/BurntSushi/ripgrep"

[source]
type = "github"
owner = "BurntSushi"
repo = "ripgrep"

[dependencies]
# Simple dependency (any version)
"fd" = "*"

# Version-constrained dependency
"libc" = ">=0.2.0"

# Optional dependency (prefixed with ?)
"?suggested-dep" = "^1.0"

[architectures]
supported = ["x86_64", "aarch64"]

[checksum]
sha256 = "e5f0b2a4c1d3f..."
```

## TOML Format (flat)

```toml
name = "ripgrep"
version = "14.1.0"
description = "Blazingly fast search tool"
dependencies = ["fd", "libc >=0.2.0", "?suggested-dep ^1.0"]
architectures = ["x86_64", "aarch64"]
sha256 = "e5f0b2a4c1d3f..."
download_url = "https://github.com/BurntSushi/ripgrep/releases/download/14.1.0/ripgrep-x86_64.tar.gz"

[source]
GitHub = { owner = "BurntSushi", repo = "ripgrep" }
```

## JSON Format

Both layouts work in JSON too — this example mixes the nested `source` with a
flat `dependencies` array, which is fine:

```json
{
  "name": "ripgrep",
  "version": "14.1.0",
  "description": "Blazingly fast search tool",
  "author": "Andrew Gallant",
  "repository": "https://github.com/BurntSushi/ripgrep",
  "source": {
    "type": "github",
    "owner": "BurntSushi",
    "repo": "ripgrep"
  },
  "dependencies": [
    "fd",
    "libc >=0.2.0",
    "?suggested-dep ^1.0"
  ],
  "sha256": "e5f0b2a4c1d3f..."
}
```

## Sources

`[source] type = "..."` selects where the package comes from:

| `type` | Extra fields | Notes |
|---|---|---|
| `github` | `owner`, `repo` | `owner`/`repo` may be omitted when `repository` (or a source `url`) is a github.com URL; `repo` otherwise defaults to `name` |
| `chocolatey` (`choco`) | `feed_url` (or `url`) | Defaults to `https://community.chocolatey.org/api/v2` |
| `baller` (`registry`) | `url` | Required |
| `system` | `manager` | One of `apt`, `dnf`, `pacman`; Linux only |
| `cargo` (`crate`) | `crate_name` (or `name`) | Defaults to the package `name`; installed with `cargo install` |

Omitting `[source]` entirely leaves the default GitHub source, in which case the
manifest needs a `download_url`. An unknown `type` is a parse error.

The equivalent flat spelling uses the tagged variant name directly —
`[source] GitHub = { owner = "..", repo = ".." }`, `BallerRegistry = { url = ".." }`,
`Chocolatey = { feed_url = ".." }`, `System = { manager = ".." }`, or
`Cargo = { crate_name = ".." }`.

## Grouped Tables

| Nested | Flat equivalent |
|---|---|
| `[checksum] sha256 = ".."` | `sha256 = ".."` |
| `[checksum] algorithm = "SHA512"` | `hash_algorithm = "SHA512"` |
| `[architectures] supported = [..]` | `architectures = [..]` |
| `[dependencies]` name → constraint table | `dependencies = ["name constraint"]` |

A flat field wins when both are present (`sha256` beats `[checksum] sha256`).

## Fields

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Package name (lowercase, no spaces) |
| `version` | string | yes | SemVer version string |
| `description` | string | no | Short package description |
| `author` | string | no | Author or maintainer name |
| `repository` | string | no | Source repository URL |
| `download_url` | string | no | Direct download URL for archive |
| `sha256` | string | no | Expected SHA-256 hash of archive |
| `dependencies` | array | no | List of dependency strings |
| `architectures` | array | no | Supported CPU architectures |

## Dependency Strings

Each entry in the `dependencies` array is a string with the format:

```
[?]<package_name> [<version_constraint>]
```

- `?` prefix marks the dependency as optional
- `<package_name>` is the registry name
- `<version_constraint>` uses semver syntax (optional — defaults to `*`)

In the `[dependencies]` table form the same string is assembled from the key and
its value, so `"?suggested-dep" = "^1.0"` and `"?suggested-dep ^1.0"` are
identical. A constraint of `"*"` (or an empty string) is dropped.

### Version Constraint Syntax

| Syntax | Example | Meaning |
|---|---|---|
| `*` | `*` | Any version |
| `1.2.3` | `1.2.3` | Exactly version 1.2.3 |
| `^1.2.3` | `^1.0.0` | Compatible with 1.0.0 (≥1.0.0, <2.0.0) |
| `~1.2.3` | `~1.2.0` | Approximately 1.2.0 (≥1.2.0, <1.3.0) |
| `>=1.0.0` | `>=2.0` | At least 2.0 |
| `>1.0.0` | `>3.0` | Strictly greater than 3.0 |
| `<2.0.0` | `<2.0` | Less than 2.0 |
| `<=2.0.0` | `<=1.9` | At most 1.9 |
| `>=1.0 <2.0` | `>=1.0 <2.0` | Range |

### Examples

```
"libc"
"libc >=0.2.0"
"serde ^1.0"
"?suggested-dep"
"?optional-tool >=2.0"
```
