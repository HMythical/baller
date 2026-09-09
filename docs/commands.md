# CLI Reference

## Overview

```
baller <COMMAND> [ARGS] [FLAGS]
```

All commands are sports-themed. Colored output via the `colored` crate.

---

## Global Flags

These work on every subcommand and may appear before or after it —
`baller -y eject fd` and `baller eject fd -y` are the same command.

| Flag | Description |
|------|-------------|
| `--yes`, `-y` | Skip confirmation prompts (`eject`, `sweep`, `substitute`) |
| `--quiet`, `-q` | Suppress progress bars and step-by-step output |
| `--verbose`, `-v` | Increase output detail: resolved URLs, source chains and cache paths on stderr. `roster` prints full detail blocks |
| `--json` | Emit machine-readable JSON instead of formatted text; implies quiet |
| `--no-hooks` | Skip every pre/post install, eject and update hook |
| `--no-color` | Disable colored output (useful in CI) |
| `--config <DIR>` | Use an alternate baller directory; db, cache and hooks all derive from it |

`--json` prints a single JSON document on stdout and suppresses every other
print, so command output stays parseable.

Progress lines and verbose detail are written to **stderr**; stdout carries
only the data a command produces. `-v` raises the detail level, `-q` and
`--json` silence it entirely, so `baller --json roster > out.json` yields a file
holding nothing but JSON.

---

## draft — Install a Package

```
baller draft <package_name> [--version <V>] [--source <S>] [--no-deps] [--dry-run] [-f]
```

Fetches the package and all its transitive dependencies from the registry chain,
downloads, verifies the package hash (SHA-256 for GitHub/Baller, SHA-512 for
Chocolatey), extracts, symlinks the binary, and records in the SQLite database.

**Flags:**
| Flag | Description |
|------|-------------|
| `--version <V>` | Pin an exact version. GitHub and Chocolatey only — the Baller registry does not support pinning yet, and system and cargo packages always install the latest |
| `--source <S>` | Resolve from one registry (`github`, `baller`, `chocolatey`, `system`, `cargo`) instead of the configured chain. `system` is Linux-only; `cargo` needs a cargo toolchain on `PATH` |
| `--no-deps` | Install the root package alone |
| `--dry-run` | Print the resolved plan (every package, version, source and action) and change nothing |
| `--force`, `-f` | Reinstall even when the package is already on the roster (without it, installed packages are skipped) |

`--version` and `--source` combine: `--source github --version 14.1.0` pins
against GitHub alone. Dependencies still resolve through the normal chain.

```
$ baller draft ripgrep --version 14.1.0 --dry-run
Dry run ripgrep v14.1.0 from chocolatey
  • ripgrep v14.1.0 (root, install)
Note nothing was installed
```

### Installation Modes

The `draft` command follows one of four code paths depending on the package
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

**Cargo mode** (`PackageSource::Cargo`): the crate is compiled and installed
by `cargo install` into `~/.cargo/bin` — without `sudo`, since the install is
user-local. As with system mode, nothing is downloaded, extracted, or
symlinked by BALLER; it records the package for tracking and still runs the
pre-install and post-install hooks.

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
baller eject <package_name> [-f] [--purge] [--no-orphans] [--keep-bin]
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
| `--force`, `-f` | Eject even when the package is frozen |
| `--purge` | Also delete the cached download archive, not just the extracted directory |
| `--no-orphans` | Leave orphaned dependencies installed |
| `--keep-bin` | Drop the roster entry but leave the linked binary in place |

**Example:**
```
$ baller eject ripgrep
Are you sure you want to eject ripgrep? [y/N] y
Removing unused dependency regex-automata...
Ejected ripgrep

$ baller eject --yes ripgrep
Ejected ripgrep
```

**Frozen packages** cannot be ejected — thaw them first with `freeze`, or pass
`--force`. If the package is not installed, a `PackageNotFound` error is returned.

---

## roster — List or Search Packages

```
baller roster [package_name] [--frozen] [--source <S>] [--outdated] [--remote]
```

**Flags:**
| Flag | Description |
|------|-------------|
| `--frozen` | Only show frozen packages |
| `--source <S>` | Only show packages installed from `github`, `baller`, `chocolatey`, `system` or `cargo` |
| `--outdated` | Check the registries and list packages with a newer version available, without updating |
| `--remote` | Skip the local roster and search registries directly (needs a search term) |
| `--verbose`, `-v` | *(global)* Print the full detail block for every listed package |
| `--json` | *(global)* Emit the roster, detail view, outdated report or search results as JSON |

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
baller freeze [package_name] [--freeze | --thaw] [--all] [--list]
```

Toggles the frozen flag on a package. Frozen packages cannot be updated or ejected.

**Flags:**
| Flag | Description |
|------|-------------|
| `--freeze` | Freeze explicitly instead of toggling — a no-op on an already frozen package |
| `--thaw` | Thaw explicitly instead of toggling. Conflicts with `--freeze` |
| `--all` | Apply to every installed package. Requires `--freeze` or `--thaw`, since toggling everything is ambiguous |
| `--list` | Print the frozen packages and exit |

The package name is optional when `--all` or `--list` is given, and required
otherwise.

**Example:**
```
$ baller freeze fd
Frozen fd

$ baller freeze fd
Thawed fd (unfrozen)

$ baller freeze fd --freeze
Frozen fd

$ baller freeze fd --freeze
Note fd is already frozen

$ baller freeze --all --thaw
Thawed fd thawed (unfrozen)
Done 1 package(s) thawed

$ baller freeze --list
Empty No frozen packages
```

---

## substitute — Swap Packages

```
baller substitute <old_package> <new_package> [--keep-old] [--dry-run] [--no-deps]
```

Installs the new package (including all resolved dependencies), then removes
the old one. If the old package is not in the roster, it is skipped with a
warning. Hooks run for both the install (pre-install / post-install for each
resolved package) and the removal (post-eject for the old package).

**Substitute prompts for confirmation by default**, like `eject` and `sweep`.
Pass the global `--yes`/`-y` to skip the prompt.

**Flags:**
| Flag | Description |
|------|-------------|
| `--keep-old` | Install the new package but leave the old one on the roster. Also lifts the frozen check, since nothing is removed |
| `--dry-run` | List what would be installed and ejected, then stop |
| `--no-deps` | Install the new root package alone |

**Example:**
```
$ baller substitute ripgrep hound
Are you sure you want to substitute ripgrep with hound? [y/N] y
Substituting ripgrep with hound...
  ... (download, install hound) ...
  ... (eject ripgrep) ...
Done Substitution complete: ripgrep -> hound

$ baller substitute ripgrep hound --dry-run
Dry run substitute ripgrep with hound v0.3.0
  • install hound v0.3.0
  • eject ripgrep
Note nothing was changed
```

---

## sweep — Clean Cache

```
baller sweep [--all] [--dry-run] [--threshold <SIZE>]
```

**Sweep is archives-only by default.** It deletes the downloaded archive files
in `~/.baller/cache/` and leaves the extracted `<name>-<version>` directories
alone — those hold the binaries that installed packages are linked against.
Pass `--all` for the old full-wipe behavior.

Shows the size in human-readable format (e.g. `45.2 MB`) before prompting.

**Flags:**
| Flag | Description |
|------|-------------|
| `--all` (alias `--purge-extracted`) | Also delete extracted packages — the pre-existing full wipe. This breaks installed binaries until they are re-drafted |
| `--dry-run` | List the archives (and, with `--all`, the directories) that would be removed |
| `--threshold <SIZE>` | Only sweep when the cache exceeds this size. Accepts `50MB`, `1.5gb`, `512k` or a bare byte count |
| `--yes`, `-y` | *(global)* Skip the confirmation prompt |

**Example:**
```
$ baller sweep --dry-run
Sweeping would remove 3 archive(s) (45.2 MB)
  • ~/.baller/cache/https___github.com_..._ripgrep.zip
  ...

$ baller sweep
Are you sure you want to clear 45.2 MB of cached archives? [y/N] y
Done Cleared 3 archive(s) (45.2 MB)
Note kept 2 extracted package(s) — pass --all to remove them too

$ baller sweep --threshold 100MB
Sweeping cache is 45.2 MB, below the 100.0 MB threshold — nothing swept

$ baller sweep --all --yes
Done Cleared 3 archive(s) and 2 extracted package(s) (61.7 MB)
```

---

## update — Update All Non-Frozen Packages

```
baller update [packages...] [--check] [--include-frozen]
```

**Flags:**
| Flag | Description |
|------|-------------|
| `[packages...]` | Update only the named packages. No arguments updates everything. An unknown name is a `PackageNotFound` error |
| `--check` (alias `--dry-run`) | Report which packages are stale without updating them |
| `--include-frozen` | Update frozen packages too, instead of skipping them |
| `--json` | *(global)* Emit the updated / stale / up-to-date / failed lists as JSON |

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

$ baller update fd --check
Stale fd: 8.7.0 -> 8.8.0
Done 1 package(s) can be updated — run baller update to apply
```

---

## build — Assemble from a Manifest

```
baller build <path> [--dry-run] [--no-deps] [--install-dir <DIR>] [-f] [--source <S>]
```

Assembles a package from a local manifest instead of resolving a name through
the registry chain. `<path>` is either a manifest file or a directory — a
directory resolves `baller.toml` first, then `baller.json`. A directory that
holds no manifest but does hold a `Cargo.toml` is compiled from source instead;
see [Build a Rust project from source](#build-a-rust-project-from-source).

**Flags:**
| Flag | Description |
|------|-------------|
| `--dry-run` | Parse and validate the manifest, print the plan, and stop before any download |
| `--no-deps` | Ignore the manifest's declared dependencies, so none are recorded |
| `--install-dir <DIR>` | Link the binary into this directory instead of the platform default |
| `--force`, `-f` | Build over a package that is already on the roster (otherwise that is an error) |
| `--source <S>` | Override the manifest's source before resolving. Clears any manifest `download_url` so the new source is actually consulted. `github` needs a github.com `repository` URL in the manifest; `system` is Linux-only; `cargo` needs a cargo toolchain on `PATH` |

Both manifest layouts parse: the flat form and the nested form documented in
[docs/manifest.md](manifest.md).

The pipeline mirrors `draft`:

1. Resolve and parse the manifest, then validate that `name` and `version` are present.
2. Run the `pre_install` hook.
3. Resolve the package source:
   - `download_url` present → download it directly.
   - GitHub source → fetch the latest release to fill in the download URL (and
     the resolved version, which is printed when it differs from the manifest).
   - Chocolatey source → resolve the `.nupkg` from the manifest's `feed_url`.
   - Baller registry source → resolve from the manifest's registry `url`.
   - System source → install via the native package manager (Linux only).
4. Verify the checksum when the manifest carries one, extract the archive, and
   locate the binary.
5. Link the binary (symlink on Linux, `.exe` copy on Windows).
6. Record the package in the database with `manifest_path` set and
   `user_installed = true`, then run the `post_install` hook.

```
$ baller build ./mytool
Building C:\dev\mytool\baller.toml...
Scouting latest release of owner/mytool...
Version 1.2.0 -> 1.3.0
  Downloading mytool-x86_64.zip [===========>] ...
Linked symlinked to C:\Users\me\AppData\Local\baller\cache\mytool-1.3.0\mytool.exe
Done mytool v1.3.0 built!
```

Manifest dependencies are recorded in the database but are **not** installed by
`build`; draft them separately.

Errors are explicit: manifest not found, no `baller.toml`/`baller.json` in the
directory, missing required fields, an unknown source type, no resolvable
download URL, a checksum mismatch, or an existing installation without `--force`.

### Build a Rust project from source

When `<path>` is a directory with no `baller.toml`/`baller.json` but with a
`Cargo.toml`, `build` compiles the crate and installs the binary it produces:

1. Read `package.name`, `package.version` (defaults to `0.0.0`) and the first
   `[[bin]]` name (defaults to the package name) out of `Cargo.toml`.
2. Run `cargo build --release` in the project directory.
3. Locate the artifact in `target/release` (the bin name, then the same name
   with `-` swapped for `_`, plus `.exe` on Windows).
4. Link it into the platform default bin directory — `~/.local/bin` on Linux,
   `%LOCALAPPDATA%\baller\bin` on Windows — or into `--install-dir`.
5. Record the package with `source = cargo`, `manifest_path` set to the
   `Cargo.toml` and `user_installed = true`, then run the `post_install` hook.

```
$ baller build ./mytool
Building /home/me/dev/mytool/Cargo.toml...
Compiling cargo build --release...
Linked symlinked to /home/me/dev/mytool/target/release/mytool
Done mytool v1.3.0 built!
```

Dependencies are resolved by cargo while compiling, so none are recorded and
`--no-deps` does not apply; `--source` does not apply either, because the
sources are the ones on disk. Both are rejected with an explicit error.
`--dry-run`, `--force`, `--install-dir`, `--json` and `--quiet` all behave as
they do for a manifest build.

Only single-crate projects are supported — a workspace root (a `Cargo.toml`
with no `[package]` name) is an error, and so is a build that leaves no
matching binary in `target/release`.

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

Destructive commands (`eject`, `sweep`, `substitute`) prompt for confirmation
unless the global `--yes` / `-y` flag is passed:

```
$ baller eject myapp
Are you sure you want to eject myapp? [y/N] y
Ejected myapp

$ baller eject --yes myapp
Ejected myapp
```
