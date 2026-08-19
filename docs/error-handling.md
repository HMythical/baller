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
