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

- **`baller referee` is now a command group: `audit`, `check`, `scan`, `cache`, `config` and `sbom`, with `--fail-on`, SARIF/Markdown export and a CycloneDX SBOM**
  The single read-only audit command is split into subcommands, each exposing a slice of the Referee service that already existed. Backward compatibility is preserved: `baller referee [PACKAGE] [--refresh] [--no-scan]` still runs the audit with identical output and JSON shape; the bare-form arguments conflict with a subcommand, so `baller referee fd` audits `fd` and `baller referee audit fd` names the verb explicitly. `audit` (Phase A + Phase B) and `check` (Phase A only — the `audit --no-scan` path as its own verb) accept several package names and a new `--fail-on block|warn` that exits 1 when any package reaches that level through either phase — an advisory banded by the install gate's own `GateOutcome` thresholds, or a re-scan finding (`block`: a block-severity finding, `warn`: any finding), so a package the install gate would reject cannot pass a CI audit just because its advisory data is clean; the report is printed first, the audit stays read-only, and the failure surfaces as a new `BallError::RefereeAuditFailed` on stderr naming each package with the phase that tripped it (`alpha v1.0.0 (advisory), epsilon v5.0.0 (scan)`), so a `--json` document on stdout stays parseable. `unverified`/`unknown` packages and swept artifacts never trip `--fail-on`. `audit --format json|markdown|sarif [--out FILE]` renders the report through new pure writers in `src/security/export.rs`: SARIF 2.1.0 (one rule per advisory id, per `scan/<rule>` and `referee/unverified`; one result per advisory, finding and unverified package; advisories levelled on their own CVSS — `block`→`error`, `warn`/unscored→`warning`, `pass`→`note`) and a Markdown table-plus-details document. Without `--out` the format replaces the table on stdout; with it the file is written and stdout keeps the normal output; `--out` requires `--format`. `scan [--fail-on block|warn]` re-scans extracted trees with no advisory lookup, reports `not on disk` distinctly from `clean`, and can gate a Phase-B-only CI step on finding severity. `cache [--status | --clear | --prune <DAYS>]` makes the verdict cache a first-class surface, backed by new `DbManager::referee_cache_stats()` (rows and newest `checked_at` per ecosystem) and `referee_cache_prune_older_than(days)`. `config` prints the `[referee]` settings in effect (`enabled` accounts for `--no-referee`) and shows the VirusTotal key only as `set`/`unset`. `sbom [--out FILE]` emits a CycloneDX 1.5 JSON inventory built from the roster alone — one component per installed package with `purl` (Cargo/GitHub), SHA-256 hash, distribution and VCS references and a `baller:source` property, plus `dependencies` from the `package_dependencies` table (edges to packages baller did not install are left out); SPDX and license data remain out of scope. `cache`, `config` and `sbom` work with Referee disabled; `audit`, `check` and `scan` still refuse. `src/commands/referee.rs` became the `src/commands/referee/` module (`mod.rs` dispatcher plus `audit`, `check`, `scan`, `cache`, `config`, `sbom`, `export`); `CommandTypes::Referee` now wraps `RefereeArgs` with an optional `RefereeSub`. `baller help referee` documents the whole tree and a new help test enforces that every clap `referee` subcommand is described there. Tests cover subgroup and back-compat parsing, flag conflicts and invalid bands/formats, cache stats and pruning, the SARIF/Markdown/CycloneDX writers, `--fail-on` against the mock-OSV integration harness, and scan-finding `--fail-on` for both `audit` and `scan`. Documented in [docs/referee.md](docs/referee.md) (the command group, export formats and `--fail-on` exit codes; "not an SBOM" removed from the non-goals), [docs/commands.md](docs/commands.md), [docs/error-handling.md](docs/error-handling.md) and [docs/architecture.md](docs/architecture.md).

- **`build` compiles and installs Rust projects from source** (Refs #9)
  `baller build <dir>` now falls back to a Cargo build when the directory holds no `baller.toml`/`baller.json` but does hold a `Cargo.toml`: the crate name, version and first `[[bin]]` name are read from the manifest, `cargo build --release` runs in the project directory, and the artifact found in `target/release` is linked into the platform default bin directory (`~/.local/bin` on Linux, `%LOCALAPPDATA%\baller\bin` on Windows) or into `--install-dir`. The package is recorded with `source = cargo` and the `Cargo.toml` as its manifest path, and the `pre_install`/`post_install` hooks run as they do for a manifest build. `--dry-run`, `--force`, `--install-dir`, `--json` and `--quiet` are supported; `--source` and `--no-deps` are rejected because cargo resolves the project's dependencies itself. Manifest-driven builds are unchanged.

- **Referee — opt-in freshness bound for cached verdicts (`cache_ttl_days`)**
  The verdict cache has never expired, so an advisory published after a package version was first checked stayed invisible to `draft` / `update` / `substitute` on every later install of that version. A new `[referee] cache_ttl_days` setting bounds how old a cached `clean` verdict may be before OSV is asked again. It is **opt-in**: unset keeps today's durable cache byte-for-byte; `0` re-queries every `clean` verdict on every install (CI). Negative and non-numeric values are rejected at parse time.
  - **`vulnerable` rows are exempt** — re-querying could only confirm the block, so a cached block is used whatever its age. The block reason now names the cached `checked_at`, and `draft --dry-run` prints the verdict's age, so a refusal on old data reads as such.
  - **Fail-open interaction** — a stale `clean` is never acted on. If the re-query cannot reach OSV, the package is `unverified` (fail-open) or the plan is refused (fail-closed), exactly as for an uncached package.
  - **New output** — JSON identities carry `cached` and `checked_at`; package reports carry `all_cached` and `oldest_checked_at`; the Markdown export gains a `Checked` column (plus a footnote when any verdict was replayed); SARIF results carry `cached` and `checkedAt` in `properties`. `referee config` shows `cache_ttl_days` (`off` when unset) and `referee cache --status` reports how many `clean` rows the TTL has aged out (`cache_ttl_days` and `stale` in `--json`).
  - No schema migration, no new `Verdict` variant and no new error variant. Freshness is strict: a row written in the same second as a `0`-TTL lookup is still re-queried.

### Fixed

- **Referee: pre-release fidelity, `build` gating, rule-table integrity, blocked-scan ordering and safe cache pruning**
  Six fixes to the Referee layer, each of which either tightens what it catches or makes what it does louder; none loosens a block.

  **Pre-releases of a vulnerable line now block where they used to report clean — a user-visible behaviour change.** Advisory matching parsed the installed version with `parse_version_flexible`, which strips pre-release metadata because a distro version is not comparable as semver. For advisories that trade is backwards: `1.2.3-rc1` collapsed to `1.2.3`, so a range with `introduced: 0` and `fixed: 1.2.3` appeared to exclude the rc, which was reported `clean` while sitting inside the vulnerable window. A new `parse_advisory_version` (`src/security/ranges.rs`) keeps a strict-semver pre-release and falls back to the lossy parser for everything else; range *bounds* stay on the lossy parser so published ranges remain usable against distro versions. Distro ecosystems (Debian, Ubuntu, Fedora, Alpine, Red Hat, AlmaLinux, Rocky Linux, SUSE, openSUSE) are exempt: a Debian `1.2.3-7` and a Fedora `8.2.2637-20.fc36` both *parse* as semver pre-releases, and reading the seventh packaging of 1.2.3 as an rc would have blocked patched system packages — the exemption is keyed on the query's ecosystem, or on the advisory entry's own ecosystem for declared aliases. The explicit `versions` list deliberately stays lossy, because a precise comparison could only remove hits from the publisher's own enumeration. The same precise comparison now drives `update` and `roster --outdated` through a shared `is_newer_release`, so an installed `1.0.0-rc1` is finally offered `1.0.0`; system packages keep the lossy comparison there too, since semver orders `5ubuntu10` below `5ubuntu2` and would offer a downgrade as an update. In the dependency solver, a new `widen_prerelease_constraints` lets a pre-release depender whose requirement already names a pre-release accept pre-releases of its bounds (`>=1.2.0-beta, ^1.4.0` admits `1.4.0-rc1`). It appends `-0` only to `^`, `~`, `>=` and `<` comparators — on `=`, `>` and `<=` it would change meaning (`<=1.2.5-0` refuses `1.2.5`) — and leaves stable dependers and requirements with no pre-release bit-for-bit unchanged; it is applied after a successful parse, so the malformed-constraint error is untouched, and there is still no backtracking.

  **`build` is now gated by both phases — and `build --dry-run` can now touch the network.** `build` had no Referee call at all. On the manifest path the download URL is now resolved first (the registry may overwrite the manifest's version, and the gate must check the version that is installed), then Phase A gates the one package before the `--force` check and the `pre_install` hook; `build` does no dependency resolution, so its recorded `dependencies` are not checked. `--dry-run` prints the gate and any block it would hit and exits successfully, as `draft --dry-run` does — which means a dry run of a manifest without `download_url` now makes a registry call and an advisory call, where it used to touch no network. Phase B runs after extraction through the same purging `screen_artifact` wrapper as `draft`/`update`/`substitute`, and the post-install and dry-run JSON gain a `referee` key. On the Cargo-project path the crate's `crates.io` identity is gated before any compile time is spent, and Phase B scans **only the compiled binary** through new `ArtifactScanner::scan_file_at` and `Referee::screen_binary` — a walk of `target/release` would exhaust the 20,000-file budget on `deps/`/`incremental/`/`.fingerprint/` and flag scripts generated by dependency build scripts. A block there goes through a new non-purging `commands::draft::screen_binary` wrapper, because purging would delete the user's own build output; `RefereeScanBlocked` gained a `discarded` field so that error reads "the compiled binary … was left in place and nothing was linked" instead of claiming a discard. `eject` installs nothing and stays ungated. The dry-run blocked-package printer is shared as `draft::report_blocked`.

  **One broken scan rule no longer silently disables all of them.** `ArtifactScanner::new` compiled the rule table as one `RegexSet` and, on any failure, substituted an empty set with a `debug` log — invisible at the default level, and total. The whole-set compile is kept as the fast path; on failure each expression is compiled alone, the set is rebuilt from the ones that work, and every lost rule is named at `warn`. Because a `RegexSet` reports its own indices, every match index is now translated back to the table before *both* the severity lookup and the evidence lookup, which previously indexed `PATTERNS` unchecked; a wrong translation would have attributed findings to the wrong rule. A new `uncompiled_rules()` accessor and a test pin the shipped table: every rule compiles today.

  **A VirusTotal block now leads the blocked-scan error instead of trailing every warning.** Findings were sorted worst-first inside `scan_tree`, then VirusTotal's were appended after the sort. The sort is now `scan::sort_findings`, run again after the VirusTotal extension in `Referee::screen_artifact` — which also fixes the order in `referee audit`, since it reads the same result. Each bullet of the `RefereeScanBlocked` error now leads with the severity in capitals (`• BLOCK: [rule] path — evidence`); the header and `ScanFinding::describe()` are unchanged, so the Markdown, JSON and SARIF exports are too.

  **`referee cache --prune` no longer deletes vulnerable verdicts.** The prune ran an unqualified `DELETE` on age, destroying exactly the rows the TTL deliberately never expires, so a housekeeping command turned known vulnerabilities back into "no data" that an audit reports as `unknown`/`unverified`. `referee_cache_prune_older_than` is replaced by `DbManager::referee_cache_prune(days, include_vulnerable)` returning a `RefereeCachePrune` (`removed`, `removed_vulnerable`, `kept_vulnerable`): `vulnerable` rows are kept by default and the command says how many it kept; new `--include-vulnerable` (requires `--prune`) removes them and says how many went. The JSON gains `removed_vulnerable` and `kept_vulnerable`; every existing key is unchanged. The TTL and `--prune` stay independent — they now only agree on which rows each may touch.

  **Docs and help corrected where they had drifted from the code.** [docs/referee.md](docs/referee.md): the script-dialects row no longer splits one flat, un-`cfg`-gated `SCRIPT_EXTENSIONS` list into Linux and Windows columns; the disabled-Referee sentence now says only `audit`/`check`/`scan` (and the bare form) refuse while `cache`/`config`/`sbom` work either way; `unknown` is documented with all three of its production sites and `unverified` as the fail-open lookup failure; the per-command table gains both `build` paths, with why the Cargo path scans one binary and why `eject` is not gated; the "`build` is not gated" known gap is replaced; the code map lists `build.rs` and the full `referee_cache_*` set. Also updated: [docs/commands.md](docs/commands.md) (`build` pipeline and `--dry-run`, `cache` flags), [docs/error-handling.md](docs/error-handling.md), [docs/architecture.md](docs/architecture.md), and `baller help referee`, which now names `build` and `--include-vulnerable`.

- **Distro versions with zero-padded segments are no longer unreadable**
  `parse_version_flexible` normalised `2:8.1.0875-5ubuntu2` down to `8.1.0875` and then handed it to semver, which rejects leading zeros on a numeric identifier — so the whole version parsed as `None`. Every caller treats that as unparseable, which meant an apt package with a zero-padded segment could fail dependency resolution outright with "system package format not supported by semver". Leading zeros are now stripped per segment (`8.1.0875` → `8.1.875`, `1.00.0` → `1.0.0`) before the semver parse, which is also what lets Referee place a Debian version on an advisory's affected range.

- **Release workflow builds and uploads the Windows artifact** (closes #14)
  The `build-windows` job in `.github/workflows/release.yml` called `build/winbuild/build.ps1 -Profile Release`, which isn't a parameter the script accepts, so every tagged release failed before `cargo` ran and shipped without a Windows build. The job now calls `-Command release`. It also named its artifact from `needs.build-linux.outputs.version` without depending on `build-linux`, which gave an empty version. The job now reads the version from `Cargo.toml` in its own step, as the Linux job does. A new step fails the job if `build/winbuild/dist/baller.exe` is missing.

- **`clippy --all-targets -D warnings` is clean; the CI lint gate now covers tests and benches**
  `cargo clippy -- -D warnings` only linted the library and binary, so the test suite and benchmarks carried 21 warnings (`bool_assert_comparison`, `useless_format`, `needless_borrows_for_generic_args`, `unnecessary_mut_passed`, `unused_mut`) that CI never saw. All are fixed, and the `test` command in `build/linux/build.sh` / `build/winbuild/build.ps1` now runs `cargo clippy --all-targets -- -D warnings` so every target stays gated.

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

- **`sweep` propagates directory traversal errors** (PR #88)
  Cache-size traversal now fails explicitly when a directory, entry, or metadata lookup cannot be read, instead of making a threshold decision from a silent zero or partial byte total. The `FileIoErr` escapes the sweep instead of being swallowed. Closes #85.

- The case-insensitive artifact lookup in `build` passes on Windows
  `test_locate_cargo_binary_scans_for_a_case_insensitive_match` compared `PathBuf`s byte-for-byte, which fails on case-insensitive filesystems: the direct name lookup returns the requested (lowercased) spelling while the scan returns the on-disk spelling. The test now lowercases the compared file names, making it platform-agnostic.

- **The `version` subcommand is wired up and working again** (PR #35)
  Re-created the `version` command and expanded `build`, replacing stale pieces with working logic, and added local development scripts to `.gitignore`. Closes #29.

- **Referee JSON `band` honours custom thresholds**
  `PackageReport::to_json` banded every package against `RefereeThresholds::default()`, so with a custom `warn_at` / `block_at` the per-package `band` in `draft --json` and `baller referee --json` could disagree with the top-level `warn_at` / `block_at` in the same document and with the terminal report. For example, with `block_at = 3.0` a package whose risk was 3.0 was blocked but its JSON said `"band": "warn"`. `to_json` now takes the thresholds in effect, as `band` and `block_reason` already did. Output is unchanged when the thresholds are left at their defaults.

### Changed

- **The build scripts install to a per-machine *and* a per-user directory**
  `install` used to write to a single directory, and the two platforms picked different ones: Linux defaulted to `/usr/local/bin`, which needs root, while Windows defaulted to `%LOCALAPPDATA%\baller\bin`, which does not. That combination was quietly broken in both directions — on Linux a plain `./build.sh install` asked for `sudo` to reach a directory most people are not installing to, and on Windows a non-elevated install landed in an application-data folder that is not on `PATH`, so the `baller` you had been running was not the one you had just built. Both now install to two directories: `/usr/local/bin` and `~/.local/bin` on Linux, `%ProgramFiles%\baller\bin` and `%LOCALAPPDATA%\baller\bin` on Windows.

  The reason for two is that whichever comes first on `PATH` wins, so installing to only one is how a machine ends up with two `baller` binaries that disagree — the freshly built one silently shadowed by a stale one, which is the exact symptom that makes `baller --version` useless as a check (nothing in it moves off `0.1.5`). Writing to both keeps them identical and makes either one a safe fallback. A per-machine directory that cannot be written is now reported and skipped rather than aborting the run, so a plain non-elevated install still leaves a working `baller` on `PATH` and tells you to re-run with `sudo` (or an elevated PowerShell) for the other one; if *neither* can be written the command fails instead of reporting success. `install` no longer deletes the existing binary before copying over it, and `uninstall` now clears every directory `install` would have written — so it can no longer leave one of the pair behind. `BALLER_INSTALL_DIR` still overrides the pair and targets that one location, and `BALLER_SYSTEM_INSTALL_DIR` / `BALLER_USER_INSTALL_DIR` override one side each.

  `install.sh` and `uninstall.sh` had been carrying a second, hand-maintained copy of the same install logic, which is how the two drifted apart in the first place; they now delegate to `build.sh`, as do `install.ps1` and `uninstall.ps1` to `build.ps1`.

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
