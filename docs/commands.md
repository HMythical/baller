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
downloads, verifies the package hash (SHA-256 for GitHub/Baller, SHA-512 for
Chocolatey), extracts, symlinks the binary, and records in the SQLite database.

### Installation Modes

The `draft` command follows one of three code paths depending on the package
source:

**Archive mode** (GitHub, Baller Registry): download archive →
SHA-256 verify → extract → symlink binary → record in DB.

**Chocolatey mode** (Chocolatey/NuGet): download `.nupkg` archive →
SHA-512 verify (base64 hash decoded from Chocolatey API) → extract as zip →
record in DB. NuGet `.nupkg` files are treated as zip archives.

**System mode** (`PackageSource::System`, i.e. `apt` / `dnf` / `pacman`):
the package is installed in place by the native package manager under `sudo`.
No archive is downloaded, no extraction happens, no symlink is created —
BALLER only records the package in its database for tracking. The pre-install
and post-install hooks still run.

**Example — archive mode:**
```
$ baller draft ripgrep
Drafting ripgrep...
  Downloading ripgrep [=============>] 2.1 MB / 2.1 MB (5s)
  Linked to ~/.local/bin/rg
Done ripgrep v14.1.0 drafted!
```

**Example — Chocolatey mode:**
```
$ baller draft python
Drafting python...
  Downloading python [=============>] 2.9 kB / 2.9 kB (1s)
  SHA-512 verified
Done python v3.15.0-b3 drafted!
```

**Example — system mode:**
```
$ baller draft default-jdk
Drafting default-jdk...
  System installing via apt...
[sudo] password for user:
Done default-jdk v2:1.21-76 installed via apt!
```

**What happens:**
1. Fetch package metadata from registry chain
2. Resolve all transitive dependencies
3. For each dependency (in order): pre-install hook →
   - **System source**: `sudo <pm> install -y <name>` → DB insert → post-install hook
   - **Archive source**: download → hash verify → extract → symlink → DB insert → post-install hook
   - **Chocolatey source**: download `.nupkg` → SHA-512 base64 verify → extract as zip → DB insert → post-install hook
4. Install root package last

---

## eject — Uninstall a Package

```
baller eject [--yes/-y] <package_name>
```

Removes a package's binary symlink, runs hooks, cleans the cache directory,
deletes its database record, and removes any orphaned dependencies.

When ejecting a package, BALLER checks for orphaned dependencies — packages
installed as transitive dependencies that are no longer required by any other
installed package. Orphans are automatically removed.

The cache directory (`~/.baller/cache/<package>-<version>`) is cleaned after
the database entry is removed.

The post-eject hook runs before the database entry is removed, so a hook
failure leaves the package record intact for retry.

**Flags:**
| Flag | Description |
|------|-------------|
| `--yes`, `-y` | Skip the confirmation prompt |

**Example:**
```
$ baller eject ripgrep
Are you sure you want to eject ripgrep? [y/N] y
Removing unused dependency regex-automata...
Ejected ripgrep

$ baller eject --yes ripgrep
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
Descriptions are safely truncated — no multi-byte character panics.

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
or falls back to searching remote registries. Remote search results include
descriptions when available.

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

Installs the new package (including all resolved dependencies), then removes
the old one. If the old package is not in the roster, it is skipped with a
warning. Hooks run for both the install (pre-install / post-install for each
resolved package) and the removal (post-eject for the old package).

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
baller sweep [--yes/-y]
```

Removes all cached archive downloads from `~/.baller/cache/` and recreates the
directory. Shows the cache size in human-readable format (e.g. `45.2 MB`) before
prompting.

**Flags:**
| Flag | Description |
|------|-------------|
| `--yes`, `-y` | Skip the confirmation prompt |

**Example:**
```
$ baller sweep
Are you sure you want to clear 45.2 MB of cached packages? [y/N] y
Sweeping cache (45.2 MB)...
Done Cache cleaned

$ baller sweep --yes
Sweeping cache (45.2 MB)...
Done Cache cleaned
```

---

## update — Update All Non-Frozen Packages

```
baller update
```

Iterates all installed packages, skips frozen ones, checks the registry for
newer versions, and upgrades each one. Version comparison uses
`parse_version_flexible()` which handles Debian epoch prefixes, revision
suffixes, embedded tags, date-based versions, and multi-segment versions —
not just strict semver. Falls back to string comparison when both versions
cannot be parsed.

New dependencies introduced by the updated version are automatically installed.
Old extracted cache directories are cleaned before the new version is installed.

Both `BALLER_OLD_VERSION` and `BALLER_NEW_VERSION` environment variables are
available in pre-update and post-update hooks.

**Example:**
```
$ baller update
OK ripgrep is up-to-date
Fetching new dependency: libz-sys
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
Not yet implemented — returns an error with a clear message and non-zero exit code.

---

## inject — Add a Custom Command

```
baller inject <path-to-.ball-file>
```

Parses a `.ball` manifest and registers the binary it names as a baller
subcommand, stored in `injected_commands.json` under baller's config directory
(`~/.baller` on Linux, `%LOCALAPPDATA%\baller` on Windows). Afterwards
`baller <command-name>` runs that binary, forwarding every argument and the
child's exit code.

Injection is gated behind three separate confirmations, because the injected
binary later runs with the user's privileges. Answering anything but `y`/`yes`
at any prompt aborts without writing.

Before writing, `inject` rejects manifests that name a built-in command
(`draft`, `eject`, `freeze`, `roster`, `substitute`, `sweep`, `update`,
`build`, `inject`, `help`, `version`), name a binary that does not exist, or
whose command name contains whitespace. The stored path is canonicalized, so a
relative `PATH` keeps working from any directory.

At invocation time, `REQUIRE-ROOT` is enforced (running unprivileged is an
error) and every entry in `DEPENDS` must resolve on `PATH`.

### `.ball` File Format

`[SECTION]` headers with `KEY = VALUE` entries. `[COMMAND-NAME]` and `[PATH]`
are required; everything else is optional, and `VERSION` defaults to `0.1.0`.
Values may be quoted or bare, `#`/`;` start a comment, and `FLAGS-LIST` /
`DEPENDS` are comma-separated.

```
[COMMAND-NAME]
COMMAND-NAME = "my-tool"

[DESCRIPTION]
DESCRIPTION = "A helpful tool that does X"

[VERSION]
VERSION = "1.0.0"

[FLAGS]
FLAGS-LIST = "-y, --yes, -n, --no"

[AUTHOR]
AUTHOR = "HMythical"

[REQUIRE-ROOT]
ROOTPERMS = false

[DEPENDS]
DEPENDS = "python3, ffmpeg"

[PATH]
PATH = /usr/local/bin/my-tool
```

`FLAGS-LIST` and `AUTHOR` are shown in help output only; baller does not
validate the flags a binary actually accepts. See `example.ball` in the repo
root for a working file.

```
$ baller inject ./my-tool.ball
This will modify baller's runtime behavior by adding a new command. Continue? [yes/no] yes
Only inject .ball files from sources you trust: the binary they name runs with your privileges. Continue? [yes/no] yes
Final confirmation: inject 'my-tool' from /usr/local/bin/my-tool? [yes/no] yes
Done Injected 'my-tool' -> /usr/local/bin/my-tool
Run it with: baller my-tool
```

---

## help — List Commands or Describe One

```
baller help [command]
```

With no argument, prints every built-in command followed by an `Injected
commands:` section listing whatever `inject` has registered — which is why
baller ships its own `help` rather than clap's built-in one, since clap only
knows about compile-time subcommands.

With an argument, prints the usage, arguments, and notes for that one command.
Built-in names resolve first, then injected ones; an unrecognized name is an
error. For an injected command the detail view shows its version, author,
flags, dependencies, root requirement, and the binary it points at.

`baller --help` and `-h` still print clap's own summary (stdout, exit 0) and
point at `baller help` for the injected list.

```
$ baller help roster
roster - Rosters (Lists) active players on your team or searches for one

Usage: baller roster [PACKAGE_NAME]

  [PACKAGE_NAME]  Package to look up; omit to list everything installed
```

---

## Confirmation Prompts

Destructive commands (`eject`, `sweep`) prompt for confirmation unless the
`--yes` or `-y` flag is passed:

```
$ baller eject myapp
Are you sure you want to eject myapp? [y/N] y
Ejected myapp

$ baller eject --yes myapp
Ejected myapp
```
