# Manifest Format

Baller uses `baller.toml` (preferred) or `baller.json` manifest files to describe
packages. The manifest is parsed by the `ManifestParser` in `src/core/manifest.rs`
and consumed by [`baller build`](commands.md).

**Two layouts parse:** the *nested* layout below (grouped `[source]`,
`[checksum]`, `[architectures]`, `[dependencies]` tables) and the *flat* layout
that mirrors the internal `Package` struct. The parser normalizes the nested
layout into the flat model, so the two are interchangeable. Serialization always
emits the flat layout, with any [`[source.<os>]`](#platform-specific-sources)
table already resolved for the host that parsed the manifest — the other
platform's table is not carried over.

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
| `chocolatey` (`choco`) | `feed_url` (or `url`) | Defaults to `https://community.chocolatey.org/api/v2`; Windows only |
| `baller` (`registry`) | `url` | Required |
| `system` | `manager` | One of `apt`, `dnf`, `pacman`; Linux only |
| `cargo` (`crate`) | `crate_name` (or `name`) | Defaults to the package `name`; installed with `cargo install` |

Omitting `[source]` entirely leaves the default GitHub source, in which case the
manifest needs a `download_url`. An unknown `type` is a parse error.

Sources are platform-scoped: `chocolatey` serves only Windows hosts and `system`
only Linux hosts, while `github`, `baller` and `cargo` serve both. `build`
rejects a source the host cannot use **before** the dry run, before any
lifecycle hook and before anything is downloaded, with an error naming the
package, the source, the host and the fix:

```
[Error]: 'tool' uses the chocolatey source, which only serves windows hosts, so it cannot be installed on linux — declare a [source.linux] table for this platform in its manifest, or pass a different --source
```

The equivalent flat spelling uses the tagged variant name directly —
`[source] GitHub = { owner = "..", repo = ".." }`, `BallerRegistry = { url = ".." }`,
`Chocolatey = { feed_url = ".." }`, `System = { manager = ".." }`, or
`Cargo = { crate_name = ".." }`.

## Platform-specific Sources

A package that ships differently per platform declares a `[source.linux]` and/or
`[source.windows]` table beside (or instead of) the plain `[source]`:

```toml
name = "tool"
version = "1.0.0"

# The fallback, used on any platform with no table of its own
[source]
type = "github"
owner = "o"
repo = "tool"

# Declares no type: keeps the github source above and overrides the artifact
[source.linux]
download_url = "https://github.com/o/tool/releases/download/v1.0.0/tool-linux.tar.gz"
sha256 = "aaa..."

# Declares a type: replaces [source] wholesale on Windows
[source.windows]
type = "chocolatey"
```

The table for the host doing the parse is selected when the manifest is read,
so the rest of baller only ever sees one source:

- **A table that declares a source** — `type = ".."`, or the flat tagged form
  (`[source.linux.System]` with `manager = "apt"`) — **replaces** `[source]` on
  that platform. No field is inherited, so a github `url` cannot leak into a
  chocolatey `feed_url`.
- **A table that declares no source** inherits `[source]` and may only set
  `download_url`, `sha256` and `hash_algorithm`. These override the top-level
  fields (including `[checksum]`) on that platform; fields it leaves out are
  inherited.
- **No table for the host** → `[source]` is used. With no `[source]` either, the
  host gets the default source, exactly as if `[source]` had been omitted.
- A manifest with **no platform tables** parses exactly as it always did.

Every platform table is validated on every host, so a mistake fails on both
platforms rather than only the one it targets. These are parse errors:

| Manifest | Error |
|---|---|
| `[source.mac]`, `[source.win]`, `[source.Linux]` … | `unknown platform table [source.mac]: expected [source.linux] or [source.windows] …` |
| `[source.linux]` with no keys | `[source.linux] is empty: declare a source 'type', or override download_url, sha256, hash_algorithm` |
| `[source.linux]` with `manager = "apt"` but no `type` | `[source.linux] sets 'manager' without a source 'type': …` |
| `download_url = ""` (or a non-string) in a platform table | `[source.linux] 'download_url' must be a non-empty string` |
| `linux = "apt"` inside `[source]` | `[source.linux] must be a table` |

JSON manifests use the same shape: `"source": { "type": "github", …,
"linux": { … }, "windows": { … } }`.

`--source` on `build` still overrides everything, platform tables included, and
still discards the manifest `download_url`.

## Grouped Tables

| Nested | Flat equivalent |
|---|---|
| `[checksum] sha256 = ".."` | `sha256 = ".."` |
| `[checksum] algorithm = "SHA512"` | `hash_algorithm = "SHA512"` |
| `[architectures] supported = [..]` | `architectures = [..]` |
| `[dependencies]` name → constraint table | `dependencies = ["name constraint"]` |
| `[advisory]` (no flat form — see below) | — |

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
| `hash_algorithm` | string | no | Algorithm `sha256` is in: `SHA256` (default) or `SHA512` (base64, as Chocolatey publishes it) |
| `dependencies` | array | no | List of dependency strings |
| `architectures` | array | no | CPU architectures the package runs on — an enforced allow-list (see below) |
| `advisory` | table | no | Where this package's known-issue surface lives — see below |

### Architectures

`architectures` (or `[architectures] supported`) is checked against the host's
CPU before anything is installed. A host that is not listed fails with
`'tool' only supports x86_64 (its declared architectures), not this
linux-aarch64 host`, before any hook runs. Common aliases match each other:
`x86_64` = `amd64` = `x64` = `x86-64`, and `aarch64` = `arm64`. An absent or
empty list places no restriction. The same check applies to packages a
registry returns with an `architectures` list.

## Advisory Identity

Distribution shape and advisory ecosystem are not the same thing: a tool shipped
as a GitHub release may also be published as a crate, and only its author knows
that. The optional `[advisory]` section tells Referee where to look.

```toml
name = "ripgrep"
version = "14.1.1"

[advisory]
ecosystem = "crates.io"       # OSV ecosystem: crates.io, npm, NuGet, PyPI, GitHub, …
name = "ripgrep"              # optional; defaults to the package name
aliases = ["CVE-2026-1234"]   # advisory ids this package is tracked under
```

| Field | Type | Required | Description |
|---|---|---|---|
| `ecosystem` | string | yes (within the section) | The OSV ecosystem to query. Without it the section names nothing queryable and is ignored |
| `name` | string | no | The name inside that ecosystem; defaults to the package `name` |
| `aliases` | array | no | Advisory ids (CVE, GHSA, RUSTSEC, …) fetched by id and range-checked against the installed version |

The declaration is stored on the roster, so `baller referee` re-checks the
package under the same identity the install used. It is honoured in JSON
manifests and in registry-served package metadata identically. See
[referee.md](referee.md).

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
