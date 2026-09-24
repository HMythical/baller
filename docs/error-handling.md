# Error Handling

## Overview

BALLER uses the `BallError` enum for all error types. Commands propagate errors
via the `?` operator. User-facing errors are printed with `[Error]:` prefix and
the process exits with a non-zero status code.

## Error Types

| Variant | Meaning |
|---------|---------|
| `UnsupportedOs` | Running on an unsupported operating system |
| `UnsupportedCommand` | Command not yet implemented |
| `FileIoErr` | File system I/O error |
| `InvalidConfig` | Configuration or database error |
| `NetworkError` | HTTP/network failure |
| `PackageNotFound` | Package not in database or registry. For registry lookups, may include an aggregated list of every source that was tried and why it failed (e.g. `python not found. Sources tried:\n  GitHub: ...\n  Chocolatey: ...`) |
| `HashMismatch` | Hash verification failed (SHA-256 for GitHub/Baller, SHA-512 for Chocolatey). For Chocolatey packages, the base64-encoded hash from the API is decoded and compared against the file's hex digest. |
| `ExtractionFailed` | Archive extraction error |
| `DependencyCycle` | Circular dependency detected |
| `VersionConflict` | Version constraint not satisfiable |
| `PackageFrozen` | Cannot modify a frozen package |
| `PackageManagerError` | System package manager error (no PM detected, command failed, unsupported manager, wrong code path tried to download a system package, or version unparseable as semver) |
| `NoMatchingAsset` | A GitHub release has no asset built for the host platform. Carries the package, the host `<os>-<arch>`, and every asset name the release offered |
| `NoBinaryFound` | An archive extracted cleanly but contained no executable. Carries the package, version, extract directory and cached archive path |
| `RefereeBlocked` | Referee's advisory gate refused the plan. Carries every package over the block threshold with its advisories and the reason. Raised before the install loop, so nothing was downloaded, linked or recorded |
| `RefereeScanBlocked` | Referee's artifact scan refused a downloaded package. Carries the package, version and every finding. Raised after extraction and before linking; the extract directory and cached archive are both purged |
| `RefereeUnavailable` | The advisory service could not be reached. Fatal only under `fail_policy = fail-closed`; the default fail-open path reports the packages as `unverified` and continues |
| `RefereeAuditFailed` | `baller referee audit`/`check --fail-on block\|warn` found packages at or above that band. Raised after the report is printed, only to make the exit code non-zero for CI; nothing was changed |

## GitHub Source Asset Errors

A GitHub release usually ships one asset per platform, named with no agreed
convention: Rust projects use target triples (`x86_64-unknown-linux-gnu`), Go
projects use `linux_amd64`, and distro packages, checksum files and signatures
sit alongside them. `src/http/github.rs` picks one in two stages:

1. **Exclude** — by lowercased asset name: forbidden extensions (`.deb`,
   `.rpm`, `.msi`, `.sig`, `.asc`, `.sha256`, `.sum`, `.txt`, `.json`,
   `.dsc`, `.buildinfo`, …, which covers `checksums.txt`), foreign-OS tokens
   (`darwin`, `macos`, `apple`, `android`, `*bsd`, `wasm`, … plus `windows`
   on a Linux host and `linux` on a Windows host), and foreign-arch tokens
   (the arm64 family, `riscv`, `ppc64`, `s390x`, `i686`, `386`, … on an amd64
   host, and the mirror set on an arm64 host).
2. **Match** — the surviving names are scanned for the host's candidate tags,
   most specific first: full target triples (gnu before musl), then Go-style
   `os_arch` names, then bare tokens such as `x86_64` / `amd64`. Where a
   release ships both a bare binary and an archive of the same build, the
   archive wins, because only an archive can be extracted.

**There is no "first asset" fallback.** When nothing matches, the fetch fails
with `NoMatchingAsset`, which names the host platform and lists every asset the
release offered:

```
[Error]: no linux-x86_64 asset for 'Rectangle' — available: Rectangle.pkg,
Rectangle1.100.dmg, Rectangle106-100.delta (specify a different source or version)
```

Previously the code matched the literal string `linux-x86_64` — a scheme almost
no project uses — and fell back to `release.assets.first()`, so `draft` happily
downloaded a macOS `.dmg` or a `.deb` and reported success (issue #13).

If the download and extraction succeed but no executable is found in the
extracted tree, the command fails with `NoBinaryFound` rather than warning and
continuing. Every command that extracts an archive enforces this —
`draft`, `build`, `substitute` and `update` (both when it installs a newly
declared dependency and when it upgrades a package) all route the case through
`Downloader::no_binary_error`:

```
[Error]: no binary found in extracted package 'ripgrep' v15.2.0 — expected an
executable for linux in ~/.baller/cache/ripgrep-15.2.0 (archive: ~/.baller/cache/...)
```

The package is **not** recorded in the roster, no symlink is created, and both
the extract directory and the cached archive are removed so a retry starts
clean.

Both variants are **hard errors**. `PackageNotFound` is treated as skippable
during dependency resolution (`core::dep_solver`) only when every package that
depends on the missing name is itself system-sourced — the Debian/RPM
virtual-package window. Any other unresolvable dependency is recorded in
`ResolveResult.unresolved`, and `draft`/`substitute` abort with
`UnresolvedDependencies` (`update` stays best-effort and warns per name), so
a dependency with no usable asset can never be silently dropped from an
install. Inside the multi-source fallback chain, however, a `NoMatchingAsset`
from GitHub is folded into the aggregated `PackageNotFound` message like any
other per-source failure:

```
[Error]: package not found: rxhanson/Rectangle not found. Sources tried:
  BallerRegistry: network error: ...
  System: package manager error: apt-cache exited with status exit status: 100
  Cargo: package manager error: cargo is not installed on this host
  GitHub: no linux-x86_64 asset for 'Rectangle' — available: Rectangle.pkg, ...
```

Pin the source (`--source github`) to see the error on its own.

## Registry Source Aggregation

When `RegistryClient::fetch_package` falls through every source in
`source_order` without a hit, the resulting `PackageNotFound` error message
contains the per-source failure details:

```
python not found. Sources tried:
  GitHub: package not found: https://api.github.com/...
  BallerRegistry: network error: HTTP 404
  Chocolatey: network error: HTTP 406
  System: package not found: python
```

This makes it much easier to diagnose why a particular package is not
resolvable — earlier errors are no longer silently swallowed.

## System Package Errors

System package installation (`install_system_package`) and the
`PackageSource::System` path of `draft` use `BallError::PackageManagerError`
exclusively. Common messages:

- `"'foo' is a system package — use native package manager"` — when
  `Downloader::download_and_extract` is called on a system package
- `"apt install of 'foo' exited with status exit code: 1"` — when the
  native PM fails
- `"unsupported system package manager: <name>"` — for managers other
  than apt/dnf/pacman
- `"unparseable version '2:1.21-76' for 'foo' (system package format not
  supported by semver)"` — when `parse_version_flexible` cannot normalize
  a system package version
- `"invalid base64 hash: ..."` — when a Chocolatey package's base64-encoded
  hash cannot be decoded

## Cargo Package Errors

Crate installation (`install_cargo_package`) and the `PackageSource::Cargo`
path of `draft` use `BallError::PackageManagerError` exclusively. Common
messages:

- `"'foo' is a cargo package — use cargo install"` — when
  `Downloader::download_and_extract` is called on a cargo package
- `"cargo install of 'foo' exited with status exit code: 1"` — when cargo
  fails to build or install the crate
- `"cargo is not installed on this host"` — when the Cargo source is queried
  on a host with no cargo toolchain
- `"--source cargo needs a cargo toolchain on PATH"` — when `build --source
  cargo` is used on such a host
- `"cargo packages always install the latest available version — cannot pin
  'foo'"` — when `--version` is combined with the Cargo source

See [docs/cargo-registry.md](cargo-registry.md) for the full error table.

## What a Failed Install Leaves Behind

A package that fails one of the hard errors above leaves **nothing** of itself
on disk or in the database: no symlink, no roster row, no extract directory and
no cached archive. A retry therefore re-downloads from scratch rather than
reusing a half-usable cache.

Per command:

| Command | On `NoBinaryFound` |
|---------|--------------------|
| `draft` | The package is not linked and not recorded. Packages installed earlier in the same run stay installed |
| `build` | Nothing is linked (neither the platform default nor `--install-dir`) and no row is written, so the manifest can be fixed and rebuilt |
| `substitute` | Fails **before** the old package is ejected, so the roster keeps the working package rather than losing it for an unusable replacement |
| `update` | The upgrade aborts. `update` prunes the old extract directory before unpacking the new archive, so the previously linked binary is gone — re-run `draft --force` (or `update` once upstream ships a usable asset) to restore it |

## Referee Errors

`RefereeBlocked` is raised by the Phase A gate, which runs on the whole resolved
plan **before** the install loop and before the `pre_install` hook. Nothing has
been fetched at that point, so the block is atomic by construction: a blocked
dependency in the middle of a five-package plan leaves zero symlinks, zero
roster rows and an empty cache.

`RefereeScanBlocked` is raised after an archive is extracted and before its
binary is linked. The extract directory and the cached archive are both removed
through the same cleanup `NoBinaryFound` uses, so a retry re-downloads rather
than reusing a rejected archive.

| Command | On `RefereeScanBlocked` |
|---------|-------------------------|
| `draft` | The package is not linked and not recorded; earlier packages in the same run stay installed |
| `substitute` | Fails before the old package is ejected |
| `update` | The scan runs **before** the old extract directory is pruned, so a rejected upgrade leaves the previously linked binary intact and still runnable |

Both carry the advisories or findings that caused them, and `RefereeBlocked`
names the escape hatches (`--no-referee`, or raising `referee.block_at`).
`RefereeUnavailable` is only fatal under `fail_policy = fail-closed`.
`RefereeAuditFailed` is not an install failure at all: it is how
`baller referee --fail-on` reports, through the exit code, that the audit found
something at or above the chosen band.
See [referee.md](referee.md).

## Rollback on Partial Failure

When `draft` or `substitute` installs multiple packages and one fails mid-way,
all packages installed during the current session are cleaned up:
- Symlinks are removed
- Database entries are deleted
- The original error is returned after cleanup

This ensures the system is never left in a partially-installed state.

## Confirmation Prompts

Destructive operations require user confirmation unless `--yes` / `-y` is provided:

- **`eject`** — removes a package and its orphaned dependencies
- **`sweep`** — clears cached archives (and, with `--all`, extracted packages)
- **`substitute`** — installs the replacement and removes the old package

## Orphan Dependency Cleanup

When ejecting a package, BALLER automatically removes dependencies that are no
longer needed by any remaining installed package. A dependency is considered
orphaned if:

1. It was not explicitly installed by the user (`user_installed = false`)
2. No other installed package lists it as a dependency

Orphaned packages are removed with a warning message. Their symlinks and
database entries are cleaned up.

## Hook Error Handling

If a lifecycle hook (pre/post install, eject, or update) fails with a non-zero
exit code, the command is aborted and the error is returned. For `eject`, the
post-eject hook runs before the database entry is removed, so a hook failure
leaves the package record intact for retry.
