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

- **Referee — a package security layer that runs before and after every install**
  B.A.L.L.E.R. now checks what it is about to install. Referee runs in two phases and is on by default.

  **Phase A, the advisory gate**, runs in `draft`, `update` and `substitute` on the *fully resolved plan* — the requested package and every dependency — before the install loop starts and before the `pre_install` hook fires. Each package is expanded into the identities public advisory data actually knows it by, all of them are queried in one batched request, and the **worst** answer decides the package: a vulnerability found under any identity is a vulnerability in what gets installed. A pre-loop gate is what makes a block atomic — the install loop has no rollback, so blocking dependency #3 of a 5-package plan from inside it would leave #1 and #2 already symlinked and recorded. Blocking before the loop means zero symlinks, zero roster rows and an empty cache on every OS.

  **Phase B, the artifact scan**, runs after an archive is extracted and before its binary is linked, because a release backdoored last night has no advisory yet. It reads every file in the tree — text directly, binaries through a `strings`-style pass — and looks for decode-then-execute chains (`base64 -d | sh`, `[Convert]::FromBase64String` + `iex`, `certutil -urlcache`, `powershell -enc`), reverse shells (`/dev/tcp/…`, `nc -e /bin/sh`), credential exfiltration (an AWS/Azure/GitHub/npm secret name near a `curl`/`wget`/`Invoke-WebRequest` call, reads of `~/.ssh/id_*` or `.aws/credentials` followed by a send), persistence (`reg add …\CurrentVersion\Run`, `schtasks /create`, writes into `~/.bashrc`), setuid/setgid and world-writable bits on Unix, launcher and installer files a release archive has no reason to ship (`.lnk`, `.hta`, `.msi`, `.desktop`, `autorun.inf`), and small scripts whose contents read as packed data. Rules are split by confidence: patterns with no benign reading block, ones that are common in honest install scripts (`curl … | sh`) warn — a scanner that blocks ordinary release archives gets turned off, and then it protects nothing. Reading is bounded per file, per tree, by file count and by depth, and symlinks are not followed, so a hostile archive cannot turn the scan into the denial of service. On a block the extract directory *and* the cached archive are purged through the same cleanup `NoBinaryFound` uses, so a retry re-downloads rather than reusing a rejected archive. In `update` the scan deliberately runs **before** the old extract directory is pruned, so a rejected upgrade does not also cost you the version you had working.

  **Identities.** Cargo maps to `crates.io`, GitHub to `(GitHub, owner/repo)`, Chocolatey to its NuGet id — *plus* the upstream GitHub repo derived from the `project_url` baller already downloads, because NuGet advisories are filed against nuget.org ids and a Chocolatey wrapper for a tool like 7-Zip rarely has one. Without that derivation Phase A would be weakest exactly on Windows. apt maps to `Debian` and dnf to `Fedora` as best-effort `Fallback` identities; pacman has no OSV ecosystem and is reported `Unknown` rather than guessed at. Referee never infers an ecosystem from a package name — a wrong mapping attaches some other project's advisories to your install, which is worse than admitting it does not know.

  **Verdicts are four-valued, not boolean:** `clean` (asked, nothing matched), `vulnerable`, `unknown` (nothing to ask), `unverified` (could not ask). Absence of data is never reported as safety, and unchecked packages are named in a closing summary line. Severity arrives as a CVSS vector, so Referee computes CVSS v3.x and v2 base scores from the published formulas and halves them onto a 0–5 Referee Risk Index; `warn_at` (default 2.5 ≈ CVSS 5.0) and `block_at` (default 4.0 ≈ CVSS 8.0) band the result. A matched advisory carrying no severity at all **warns** — it cannot be scored, so it is never called safe, and it never blocks on a number nobody published.

  **When the advisory service is down**, the default `fail_policy = fail-open` reports the packages `unverified` and lets the install proceed; `fail-closed` refuses instead. An `unverified` verdict is never cached, so an outage cannot become a durable answer.

  New `baller referee [PACKAGE] [--refresh] [--no-scan]` audits the roster: every installed package re-checked against advisory data and its extracted tree re-scanned. It is read-only — a flagged package is on your roster because it was installed before the advisory existed, and silently ejecting it would be a worse surprise than reporting it. New global `--no-referee` skips both phases for one command; `[referee]` in `baller.conf` sets `enabled`, `warn_at`, `block_at`, `fail_policy`, `osv_base_url`, `virustotal_api_key` and `virustotal_base_url`, validated as `0 <= warn_at < block_at <= 5` at parse time. Verdicts are cached in a new `referee_cache` table keyed by `(ecosystem, name, version)` — a `clean` verdict says nothing about any other version — and a cache hit costs no network request. `--json` embeds the full report in each command's single JSON document. New `BallError` variants: `RefereeBlocked`, `RefereeScanBlocked`, `RefereeUnavailable`.

  Manifests gained an optional `[advisory]` section (`ecosystem`, `name`, `aliases`) so an author can state where their known-issue surface lives — a tool shipped as a GitHub release may also be published as a crate, and only they know that. Declared aliases are fetched by id and range-checked against the installed version. The declaration is stored on the roster (new `advisory` column, migrated in on open) so an audit re-checks a package under the same identity the install used. Registry metadata may also carry an OSV-shaped `vulnerabilities` array, consumed as authoritative with no network call — the registry is the authority on its own contents.

  Documented in [docs/referee.md](docs/referee.md). Optional hash-only VirusTotal lookup behind `virustotal_api_key`: only SHA-256 digests are sent — the request is a bodyless `GET /files/<sha256>` with the key in an `x-apikey` header — and a missing key, timeout, rate limit, network failure, unknown hash or clean report are all non-answers rather than verdicts. `virustotal_base_url` points it at a self-hosted proxy, and is what makes the hook verifiable end to end.

- **`build` compiles and installs Rust projects from source** (Refs #9)
  `baller build <dir>` now falls back to a Cargo build when the directory holds no `baller.toml`/`baller.json` but does hold a `Cargo.toml`: the crate name, version and first `[[bin]]` name are read from the manifest, `cargo build --release` runs in the project directory, and the artifact found in `target/release` is linked into the platform default bin directory (`~/.local/bin` on Linux, `%LOCALAPPDATA%\baller\bin` on Windows) or into `--install-dir`. The package is recorded with `source = cargo` and the `Cargo.toml` as its manifest path, and the `pre_install`/`post_install` hooks run as they do for a manifest build. `--dry-run`, `--force`, `--install-dir`, `--json` and `--quiet` are supported; `--source` and `--no-deps` are rejected because cargo resolves the project's dependencies itself. Manifest-driven builds are unchanged.

### Fixed

- **Distro versions with zero-padded segments are no longer unreadable**
  `parse_version_flexible` normalised `2:8.1.0875-5ubuntu2` down to `8.1.0875` and then handed it to semver, which rejects leading zeros on a numeric identifier — so the whole version parsed as `None`. Every caller treats that as unparseable, which meant an apt package with a zero-padded segment could fail dependency resolution outright with "system package format not supported by semver". Leading zeros are now stripped per segment (`8.1.0875` → `8.1.875`, `1.00.0` → `1.0.0`) before the semver parse, which is also what lets Referee place a Debian version on an advisory's affected range.

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
