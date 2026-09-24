# Changelog

All notable changes to B.A.L.L.E.R. will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
with a custom scheme:

- **Major (X)**: Breaking changes to core functionality
- **Minor (0.X.0)**: Feature additions and/or security patches
- **Patch (0.0.X)**: Bug fixes and issue resolutions

---

## [Unreleased]

### Features

- **`build` compiles and installs Rust projects from source** (Refs #9)
  `baller build <dir>` now falls back to a Cargo build when the directory holds no `baller.toml`/`baller.json` but does hold a `Cargo.toml`: the crate name, version and first `[[bin]]` name are read from the manifest, `cargo build --release` runs in the project directory, and the artifact found in `target/release` is linked into the platform default bin directory (`~/.local/bin` on Linux, `%LOCALAPPDATA%\baller\bin` on Windows) or into `--install-dir`. The package is recorded with `source = cargo` and the `Cargo.toml` as its manifest path, and the `pre_install`/`post_install` hooks run as they do for a manifest build. `--dry-run`, `--force`, `--install-dir`, `--json` and `--quiet` are supported; `--source` and `--no-deps` are rejected because cargo resolves the project's dependencies itself. Manifest-driven builds are unchanged.

### Fixed

- **`build` no longer runs the `pre_install` hook for a system-source manifest the host cannot install** (closes #66)
  The "needs a native package manager" check for `[source] type = "system"` manifests now runs before the `PreInstall` hook, using the runtime-detected host manager instead of a compile-time `cfg!(target_os = "linux")` guard inside the install path. A build on an unsupported host now fails before any user hook script executes; `--dry-run` and roster checks are unchanged.

- **Dependency resolution no longer silently swallows unresolvable dependencies** (Closes #73)
  `resolve_from` dropped any dependency whose fetch returned `PackageNotFound` — in both the already-installed and plain-fetch branches — with no warning and no record, so `draft` installed whatever subset *did* resolve and printed "Done … drafted!" as if the full set had been satisfied. The gap was invisible: `ResolveResult` had no "unresolved" concept, and nothing surfaced the difference between an optional-only hole and a missing mandatory dependency.

  `ResolveResult` now carries `unresolved: Vec<String>`, populated whenever a non-optional dependency resolves from no source. The old unconditional tolerance is scoped: a missing dependency is skipped **only when every package that depends on it is system-sourced** (a Debian/RPM virtual package the native manager handles); anything else is reported. `draft` and `substitute` abort with a new `UnresolvedDependencies` error instead of installing a subset; `update` keeps its best-effort behavior (it already re-derived `missing_deps`) but warns per unresolved name. The resolver also emits a `tracing::warn!` for every name it could not resolve, so even a tolerated gap is visible under `-v`.

- **Malformed version constraints are an error, not a silent `*`** (Closes #74)
  `parse_dependency_line` coerced any `VersionReq::parse` failure to `VersionReq::STAR`, so a typo'd constraint like `foo >=1.2..3` accepted any version. It now returns `Result` and raises `VersionConflict` naming the constraint and package; only genuinely empty constraints (`foo`, `? foo`) keep the `*` default. The error threads through `parse_dependencies` → `enqueue_deps` → the resolver, so a bad manifest line fails resolution at parse time instead of being masked downstream.

- **Dependency constraints are enforced regardless of when they are registered** (Closes #75)
  The resolver checked a package's constraint only on first pop: a package already in `resolved` was skipped wholesale, so a constraint registered later (a second path to the same transitive dep, or a dependee constraining the pinned root) was stored in `constraints` but never compared. The duplicated checks now live in one `enforce_constraints` helper, called on first resolution, on the installed-version path, on re-visiting an already-resolved package, and **at registration time** in `enqueue_deps` when the target is already resolved. A conflicting late constraint — installed `B v1.0.0` vs `C`'s `B >=2.0`, or two fetched branches converging at different depths — now raises `VersionConflict`.

- **Dependency cycles surface as errors unless they are entirely system-sourced** (Closes #76)
  `resolve_from` discarded `detect_cycles` unconditionally (`let _ =`), so a genuine `a -> b -> c -> a` cycle in any non-system graph produced an arbitrary install order instead of a `DependencyCycle` error. The three-color DFS is now `find_cycle`, and the resolver fails any cycle where at least one node is not a system package; only all-system cycles (`libc6 <-> libgcc-s1`) stay tolerated and fall back to a best-effort ordering. `topological_sort` also re-validates that its output obeys every graph edge, so a cycle can never reach the install plan silently, and packages resolved but absent from a tolerated cycle's ordering are no longer dropped from the result.

- **`draft` no longer installs another platform's release asset and calls it a success** (Closes #13)
  GitHub asset selection matched the literal string `linux-x86_64` — a naming scheme almost no project uses — with a single case-sensitive `contains`, and fell back to `release.assets.first()` when that missed. `baller draft <owner>/<repo> --source github` therefore downloaded whatever asset happened to be listed first: a macOS `.dmg`, a `.deb`, or a `checksums.txt`.

  `src/http/github.rs` now selects in two stages. Assets are **excluded** first by lowercased name — distro packages, installers, signatures, checksum and metadata files, foreign-OS tokens (`darwin`, `macos`, `apple`, `android`, `*bsd`, plus `windows` on Linux and `linux` on Windows) and foreign-arch tokens (the arm64 family, `riscv`, `ppc64`, `s390x`, `i686`, `386` on an amd64 host, and the mirror set on arm64). The survivors are then **matched** against the host's candidate tags, most specific first: Rust target triples (gnu preferred over musl), Go-style `linux_amd64` names, then bare `x86_64` / `amd64` tokens. Where a release ships both a bare binary and an archive of the same build, the archive wins, since only an archive can be extracted. The `assets.first()` fallback and `detect_arch_string()` are gone: a release with no host build is now a hard `NoMatchingAsset` error naming the platform and listing every asset offered.

  `draft` also stops recording packages it could not install. "No binary found in extracted package" was a warning that fell through to `insert_package` and exited 0, leaving a roster entry with no symlink; it is now a hard `NoBinaryFound` error, and both the `<name>-<version>` extract directory and the cached archive are removed so a retry starts clean.

  `--dry-run` gained a `Download:` line under each plan entry showing the exact asset URL (`download_url` in `--json`), mirroring `build`'s dry run; system and cargo packages print `<resolved by source>`. Because the asset is chosen during fetch, a repo with no host asset now fails the dry run too, instead of reporting a plan that cannot work. `-v` additionally logs the chosen asset's file name as it is selected.

- **`build`, `substitute` and `update` stop recording packages they could not install** (Refs #13)
  The follow-up to the `draft` fix above. All three shared the pattern issue #13 reported: `if let Some(binary_path) = &downloaded.binary_path` linked the binary when one was found and fell through to `insert_package` when one was not, so an archive that extracted without an executable still produced a roster entry, a "Done … built!" message and exit code 0. `build` announced it with `Warning no binary found in extracted package`; `substitute` and `update` said nothing at all.

  All four call sites — `build`, `substitute`, and both of `update`'s (installing a newly declared dependency, and upgrading a package) — now fail with `NoBinaryFound` before anything is linked or recorded. The cleanup that `draft` performs moved to `Downloader::no_binary_error`, so every command removes the `<name>-<version>` extract directory and the cached archive on the way out and a retry starts clean.

  Per command: `build` links nothing into the platform default or `--install-dir`; `substitute` fails **before** ejecting the old package, so a broken replacement can no longer cost you a working one; `update` aborts the upgrade (note that it prunes the old extract directory before unpacking the new archive, so restoring a package whose new release ships no usable asset needs a `draft --force`).

- The case-insensitive artifact lookup in `build` passes on Windows
  `test_locate_cargo_binary_scans_for_a_case_insensitive_match` compared `PathBuf`s byte-for-byte, which fails on case-insensitive filesystems: the direct name lookup returns the requested (lowercased) spelling while the scan returns the on-disk spelling. The test now lowercases the compared file names, making it platform-agnostic.

### Changed

- **`--verbose` is now a real verbosity switch, backed by `tracing`**
  `-v`/`--verbose` was advertised as repeatable but repeats had no effect, and `roster` was the only command that read it. The flag is now a plain boolean (the count semantics are gone, so `-vv` is rejected rather than silently ignored) and drives a `tracing_subscriber` filter installed once at startup: `-v` maps to `DEBUG`, no flag to `INFO`, and `-q`/`--json` to `ERROR`, which silences the tracer entirely.

  All tracer output goes to **stderr**, leaving stdout for data alone — a `--json` run can never interleave log lines into its JSON document. `utils::output::info` now emits at `INFO` through the tracer (so progress lines moved from stdout to stderr, with `--quiet` suppression unchanged), and a new `utils::output::debug` helper carries the verbose detail.

  `-v` now produces measurably different output for `draft`, `build` and `update`, not just `roster`: the effective registry source chain, resolved asset URLs, dependency-resolution decisions, and cache/extract directory paths. The filter is scoped to baller's own events, so `-v` does not dump the dependency tree's internals; `BALLER_LOG` overrides it for debugging, except under `-q`/`--json`, which stay silent regardless. Closes #20.

---

## [0.1.5] - 2026-09-07

### Features

- **Cargo source: install crates from crates.io** (`7b2b361`, PR #26)
  Added a `cargo` registry source (`src/http/cargo.rs`) that resolves crates through the local cargo toolchain — `cargo info` (falling back to `cargo search`) for metadata, `cargo search --limit 20` for search, and `cargo install` for installation without `sudo`. The source is wired through `--source cargo`, `[source] type = "cargo"` manifests, `draft`/`build` install dispatch, and DB provenance (`source = "cargo"`). The Linux default `source_order` is now `baller, system, cargo, github`; the Windows default (`baller, chocolatey, github`) is unchanged. New `cargo_enabled` config key defaults to `true` on Linux and `false` on Windows and can be set in `baller.conf`. Documented in `docs/cargo-registry.md` and `docs/registry.md`. Part of #8.

### Bug Fixes

- **System search stamped the wrong package manager** (`7b2b361`, PR #26)
  `search_dnf` labelled its results `apt` and `search_pacman` labelled its results `dnf`, so a package found by search on Fedora or Arch would have been installed through the wrong CLI. Both now record the manager that produced them. `pacman -Ss` parsing was also misreading the entry — it stored the description as the version and dropped the real version; the version now comes from the `repo/name version` line and the description from the indented line that follows. Part of #8.

- **The values from the configuration file should not be empty** (`46bfc5c`, PR #25)
  Added a verification that every value read from `baller.conf` is non-empty, and reorganized error checking so all configuration errors are reported at once instead of stopping at the first one. Refs #24.

- **Inject command honours the `yes` flag** (`e075bc1`, PR #28)
  The `inject` subcommand no longer blocks on its confirmation prompts when `-y`/`--yes` is passed, matching the behaviour of the other commands.

- **Confirmations no longer exit with a non-zero code** (`5caae6e`, PR #30)
  `confirm()` now returns a `Result<bool, BallError>` and propagates two new error types — `ConfirmationAborted` and `PipeRedirected` — so aborted confirmations and redirected pipes no longer surface as failures. The confirmation prompt in the `eject` subcommand was also moved below the installation check and now uses `installed.name`. Closes #17, Closes #18.

- **Invalid names are rejected in the `inject` subcommand** (`19a6aba`, PR #31)
  Command names provided to `inject` are now validated against a regular expression, so invalid names are rejected instead of being accepted. Closes #19.

---

## [0.1.2] - 2026-08-25

### Features

- **CLI expansion with global flags and manifest-driven build** (`8aa101a`)
  Added global flags (`-y`, `-q`, `--no-hooks`, `--no-color`, `--json`, `-v`, `--config`) and per-command options across `draft`, `eject`, `freeze`, `roster`, `substitute`, `sweep`, and `update`. Introduced the `build` command for assembling packages from `baller.toml`/`baller.json` manifests. Output can now be emitted as machine-readable JSON via the new `utils::output` module. Pre/post install, eject, and update hooks run from the build path. Covered by unit and integration tests across parse, manifest, freeze, roster, sweep, substitute, and output modules. Updated documentation in `docs/commands.md`, `docs/manifest.md`, `docs/registry.md`, `docs/architecture.md`, and `docs/error-handling.md`.

- **Inject command for user-extensible custom commands** (`88cd0cb`)
  Added `baller inject` command that parses `.ball` manifest files and registers external binaries as baller subcommands. Injected commands are persisted to `~/.baller/injected_commands.json` and become available immediately via `baller <command-name>`, forwarding all arguments and exit codes. Injection is gated behind three sequential warnings about the risks of running untrusted binaries. Built-in command names are reserved and cannot be overridden. New modules: `src/core/ball_parser.rs`, `src/core/injected.rs`, `src/commands/inject.rs`, `src/commands/external.rs`. 502 tests pass. Closes #11.

- **Read default GitHub owner from configuration** (`6760c9f`)
  When fetching a package from GitHub, if `default_github_owner` exists in the config file, users can specify just the package name without the owner. If not set, the owner must be specified explicitly.

### Bug Fixes

- **Fix build/install scripts: binary path resolution for release builds** (`eee0bab`, PR #23)
  Fixed issue where `./install.sh` (and `build.sh install`) could not find the release binary after running `./build.sh release`. Linux scripts now look directly in `target/$BALLER_TARGET/release/` and auto-run `run_release` if the binary is missing. Windows scripts now use the `$Target` parameter for correct binary path resolution instead of hardcoding `target\release`.

- **Remove hard-coded owner for GitHub sources** (`c8c9f0a`)
  When drafting a package from GitHub, if the owner were not specified, the URL was resolved against a hard-coded source. Now an error is raised with the expected format specified. Refs #15.

- **Fix Windows install and packaging scripts for manual builds and releases** (`81015dc`)
  Resolved issues with Windows install and packaging scripts that prevented manual builds and Windows releases from completing successfully.

- **Fix Windows native build for standby release** (`06f3d21`)
  Fixed Windows native build configuration for the new standby release process.

### Other Changes

- **Add and update CI and GitHub rules for contributors** (`9b9b0a6`)
  Implemented branch protection rulesets for `rootdev` and `deploy` branches. Configured required status checks, mandatory approvals, squash-only merge strategy, and bypass permissions for the lead maintainer. Removed path filters from CI workflows to ensure full build validation on all changes.

- **Add logo to README and assets** (`5c5a42c`)
  Added project logo to the repository assets and updated the README to display it.

---

## [0.0.1-beta-release] - 2026-07-21

Initial beta release of B.A.L.L.E.R.

### Added

- Core package management functionality for Linux and Windows
- Package installation, removal, and update commands (`draft`, `eject`, `sweep`, `update`)
- Package search and listing (`roster`, `substitute`)
- Build system for Linux and Windows environments
- SHA2 hash verification for package integrity
- Chocolatey integration for Windows packages
- Configuration file parsing (`baller.toml`/`baller.json`)
- Command-line argument parsing and help system
- CI/CD pipeline with GitHub Actions for Linux and Windows
- Contributing guidelines and documentation structure
- Project README with build and installation instructions
