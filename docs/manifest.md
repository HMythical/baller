# Manifest Format

Baller uses `baller.toml` (preferred) or `baller.json` manifest files to describe
packages. The manifest is parsed by the `ManifestParser` in `src/core/manifest.rs`.

## TOML Format

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
"fd"

# Version-constrained dependency
"libc" = ">=0.2.0"

# Optional dependency (prefixed with ?)
"?suggested-dep" = "^1.0"

[architectures]
supported = ["x86_64", "aarch64"]

[checksum]
sha256 = "e5f0b2a4c1d3f..."
```

## JSON Format

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
