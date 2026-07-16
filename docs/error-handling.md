# Error Handling

## Overview

BALLER uses the `BallError` enum for all error types. Commands propagate errors
via the `?` operator. User-facing errors are printed with `[Error]:` prefix and
the process exits with a non-zero status code.

## Error Types

| Variant | Meaning |
|---------|---------|
| `UnsupportedOs` | Running on an unsupported operating system |
| `UnsupportedCommand` | Command not yet implemented (e.g., `build`) |
| `FileIoErr` | File system I/O error |
| `InvalidConfig` | Configuration or database error |
| `NetworkError` | HTTP/network failure |
| `PackageNotFound` | Package not in database or registry |
| `HashMismatch` | SHA-256 verification failed |
| `ExtractionFailed` | Archive extraction error |
| `DependencyCycle` | Circular dependency detected |
| `VersionConflict` | Version constraint not satisfiable |
| `PackageFrozen` | Cannot modify a frozen package |
| `PackageManagerError` | System package manager error |

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
- **`sweep`** — clears the download cache

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
