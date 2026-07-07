# CLI Reference

## Overview

```
baller <COMMAND> [ARGS]
```

All commands are sports-themed. Colored output via the `colored` crate.

---

## draft — Install a Package

```
baller draft <package_name>
```

Fetches the package and all its transitive dependencies from the registry chain,
downloads, verifies SHA-256, extracts, symlinks the binary, and records in the
SQLite database.

**Example:**
```
$ baller draft ripgrep
Drafting ripgrep...
  Downloading ripgrep [=============>] 2.1 MB / 2.1 MB (5s)
  Linked to ~/.local/bin/rg
Done ripgrep v14.1.0 drafted!
```

**What happens:**
1. Fetch package metadata from registry chain
2. Resolve all transitive dependencies
3. For each dependency (in order): pre-install hook → download → hash verify → extract → symlink → DB insert → post-install hook
4. Install root package last

---

## eject — Uninstall a Package

```
baller eject <package_name>
```

Removes a package's binary symlink, deletes its database record, and runs hooks.

**Example:**
```
$ baller eject ripgrep
Ejected ripgrep
```

**Frozen packages** cannot be ejected — must be thawed first with `freeze`.
If the package is not installed, a `PackageNotFound` error is returned.

---

## roster — List or Search Packages

```
baller roster [package_name]
```

**Without arguments:** Lists all installed packages with versions.

```
$ baller roster

Active Roster (2 players):
────────────────────────────
  • ripgrep v14.1.0
    Blazingly fast search tool
  • fd v8.7.0 ❄️
    Simple, fast alternative to find
────────────────────────────
```

**With a package name:** Shows detailed info for a locally installed package,
or falls back to searching remote registries.

```
$ baller roster ripgrep

Roster - ripgrep
  Version:    14.1.0
  Source:     github
  Source Detail: BurntSushi/ripgrep
  Description: Blazingly fast search tool
  Frozen:     no
  Install Path: ~/.baller/cache/ripgrep-14.1.0
  Binary:     ~/.baller/cache/ripgrep-14.1.0/rg
```

---

## freeze — Toggle Freeze State

```
baller freeze <package_name>
```

Toggles the frozen flag on a package. Frozen packages cannot be updated or ejected.

**Example:**
```
$ baller freeze fd
Frozen fd

$ baller freeze fd
Thawed fd (unfrozen)
```

---

## substitute — Swap Packages

```
baller substitute <old_package> <new_package>
```

Installs the new package, then removes the old one. If the old package is not
in the roster, it is skipped with a warning.

**Example:**
```
$ baller substitute ripgrep hound
Substituting ripgrep with hound...
  ... (download, install hound) ...
  ... (eject ripgrep) ...
Done Substitution complete: ripgrep -> hound
```

---

## sweep — Clean Cache

```
baller sweep
```

Removes all cached archive downloads from `~/.baller/cache/` and recreates the
directory.

**Example:**
```
$ baller sweep
Sweeping cache (45.2 MB)...
Done Cache cleaned
```

---

## update — Update All Non-Frozen Packages

```
baller update
```

Iterates all installed packages, skips frozen ones, checks the registry for
newer versions, and upgrades each one.

**Example:**
```
$ baller update
OK ripgrep is up-to-date
Updating fd: v8.7.0 -> v8.8.0
  Downloading fd [===========>] ...
Done Update complete: 1 updated, 0 failed
```

---

## build — Build from Manifest (stub)

```
baller build <path>
```

Parses a `baller.toml` or `baller.json` manifest and builds the package.
Not yet implemented — prints an informational message.
