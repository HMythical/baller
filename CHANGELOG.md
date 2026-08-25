# Changelog

All notable changes to B.A.L.L.E.R. will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
with a custom scheme:

- **Major (X)**: Breaking changes to core functionality
- **Minor (0.X.0)**: Feature additions and/or security patches
- **Patch (0.0.X)**: Bug fixes and issue resolutions

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
