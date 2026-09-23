# Referee — B.A.L.L.E.R. Package Security System

Status: **Proposed** · Target milestone: `0.2.0` · Owner: B.A.L.L.E.R. core team

Referee is a package security layer for B.A.L.L.E.R. that runs **before** and
**after** package installation. It checks every package a command intends to
install (the requested package *and* every resolved dependency) against
known-vulnerability data made public on the internet, scores the severity, and
decides whether to install silently, install with a warning, or forcefully
abort the install and explain why.

The name is **Referee** — the neutral arbiter between the user and a package.

---

## Table of contents

1. [Goals and non-goals](#goals-and-non-goals)
2. [The two-phase model](#the-two-phase-model)
3. [Core abstraction: advisory identities](#core-abstraction-advisory-identities)
4. [Phase A — the advisory gate](#phase-a--the-advisory-gate)
   - [Ecosystem mapping](#ecosystem-mapping)
   - [OSV API client](#osv-api-client)
   - [Version range matching](#version-range-matching)
   - [Scoring and verdicts](#scoring-and-verdicts)
5. [Phase B — the artifact scanner](#phase-b--the-artifact-scanner)
   - [Offline heuristics](#offline-heuristics)
   - [Linux-specific checks](#linux-specific-checks)
   - [Windows-specific checks](#windows-specific-checks)
   - [Optional VirusTotal hook](#optional-virustotal-hook)
6. [Integration points](#integration-points)
7. [Configuration](#configuration)
8. [Caching](#caching)
9. [Errors](#errors)
10. [JSON output](#json-output)
11. [Impact of issue #10 — non-functional sources](#impact-of-issue-10--non-functional-sources)
12. [Runtime behaviour on Linux and Windows](#runtime-behaviour-on-linux-and-windows)
13. [Testing strategy](#testing-strategy)
14. [Milestones](#milestones)
15. [Open questions](#open-questions)

---

## Goals and non-goals

### Goals

- Surface **known** vulnerabilities (advisories/CVEs) for every package B.A.L.L.E.R.
  installs, before anything is written to the system.
- Distinguish three outcomes: **safe/unverified** (install), **warn** (install
  anyway, user informed), **block** (abort with the reason).
- Cover root packages **and** dependencies — a safe root can pull a vulnerable
  dependency, and that is exactly the case Referee exists for.
- Gate installs **without partial writes**: if anything in the plan is blocked,
  abort before creating symlinks, records, or spawning `sudo`/native installs.
- Verify the *artifact* itself after acquisition, because OSV/CVE databases
  cannot see freshly published supply-chain infections (typosquats, backdoored
  releases that ship no advisory yet).
- Degrade gracefully: an unreachable advisory database never bricks an install
  (fail-open by default), and absence of data is reported as *unknown*, never
  as *verified safe*.

### Non-goals

- Not a malware scanner replacement for Windows Defender / VirusTotal / ClamAV.
  Referee's Phase B heuristics are lightweight signals, not a full AV engine.
- Not a firewall against zero-day supply-chain attacks — heuristics will miss
  some; that is acknowledged and documented.
- No SBOM generation, no license scanning, no dependency-graph advisory
  remapping of already-installed trees (Phase A re-check happens on
  install/update only; the `referee` audit command re-scans the roster).
- Referee does not replace or wrap git hooks; it is a separate service invoked
  from commands, see [Integration points](#integration-points).

---

## The two-phase model

| Phase | When | Scope | Can block? |
|---|---|---|---|
| **A — Advisory gate** | Before the install loop, on the whole resolved plan | Every package (root + deps) | Yes — abort before anything is written |
| **B — Artifact scan** | After download + extract, before the binary is linked/recorded | Only sources that download archives (GitHub, BallerRegistry, Chocolatey) | Yes — abort with cleanup via `Downloader::no_binary_error` |

System and Cargo packages acquire no artifact B.A.L.L.E.R. controls (the native
package manager and `cargo install` handle the content), so for those sources
only **Phase A** applies.

---

## Core abstraction: advisory identities

One `Package` does not map to one vulnerability record. A single install target
can be described by several identities against public advisory data:

- a Chocolatey package is also a **NuGet** package *and* usually wraps an
  upstream tool with its own **GitHub**-ecosystem advisories;
- a Cargo crate is a **crates.io** package;
- a Debian package is a **Debian**-ecosystem package;
- a future Baller Registry package may declare aliases of its own.

Referee therefore expands each package into a **set of advisory identities** and
queries OSV for all of them in one batch:

```rust
/// One resolvable identity against the OSV API.
pub struct AdvisoryIdentity {
    pub ecosystem: String, // "crates.io", "NuGet", "GitHub", "Debian", ...
    pub name: String,      // crate name, nuget id, "owner/repo", distro source package
    pub scope: IdentityScope,
}

/// Why this identity exists — drives caching, reporting and error tolerance.
pub enum IdentityScope {
    Primary,    // the source identity itself (Cargo -> crates.io, GitHub -> GitHub)
    Derived,    // an alias discovered from package metadata (project_url, repository)
    Declared,   // a self-declared OSV identity shipped inside baller package metadata
    Fallback,   // best-effort mapping that may be wrong (distro ecosystems)
}

pub fn advisory_identities(pkg: &Package) -> Vec<AdvisoryIdentity>;
```

The **risk index for the package is the maximum across all its identities.**
If any identity reports an advisory whose score crosses the block threshold,
the package is blocked; the strongest warning wins.

This single abstraction is the architectural answer to B.A.L.L.E.R.'s weakest
advisory surface, the Chocolatey path: instead of accepting "NuGet has no record
for this id → unknown", Referee *derives* the upstream repository from the
Chocolatey metadata it already downloads and queries the GitHub ecosystem for
it. Section [Ecosystem mapping](#ecosystem-mapping) defines the expansion rules.

---

## Phase A — the advisory gate

Runs **once, up front, before any package in the plan is installed.**

### Where in the flow

In `execute_draft` (`src/commands/draft.rs`):

1. `fetch_root` resolves the root package (`draft.rs:46`).
2. `resolve_deps_with_root` produces the full plan (`draft.rs:64`).
3. **NEW: Referee Phase A gate** — expand every plan package to advisory
   identities, batch-query OSV, score, and either:
   - return `RefereeBlocked` (block threshold crossed) → **nothing installed**;
   - print warnings (warn band) and **continue**;
   - proceed silently (below the warn threshold or unknown).
4. Existing install loop proceeds (`draft.rs:89`).

**Why a pre-loop gate and not a per-package check:** the current draft loop has
no rollback — `session_packages` (`draft.rs:86`) is tracked for JSON reporting
only. If Referee blocked dependency #3 of 5 *inside* the loop, packages #1 and
#2 would already be symlinked and recorded. The pre-loop gate makes the block
atomic: an abort happens with zero writes on every OS.

### Ecosystem mapping

```rust
match pkg.source {
    PackageSource::GitHub { owner, repo }       => [(GitHub, "{owner}/{repo}", Primary)],
    PackageSource::Cargo { crate_name }          => [(crates.io, crate_name, Primary)],
    PackageSource::Chocolatey { feed_url }       => chocolatey_identities(&pkg),
    PackageSource::System { manager }            => system_identities(&pkg, manager),
    PackageSource::BallerRegistry { .. }         => registry_identities(&pkg),
}
```

| `PackageSource` | Identities | Confidence |
|---|---|---|
| `Cargo { crate_name }` | `(crates.io, crate_name)` — Primary | High. RustSec data via OSV, semver versions |
| `GitHub { owner, repo }` | `(GitHub, "{owner}/{repo}")` — Primary | High. GHSA advisories are repo-scoped, version-based, OS-agnostic |
| `Chocolatey { feed_url }` | `(NuGet, pkg.name)` — Primary; `(GitHub, {owner}/{repo})` — Derived, when `project_url` parses to github.com | Medium. See *Chocolatey coverage* below |
| `System { manager }` | `(Debian, name)` for apt; `(Fedora, name)` for dnf — Fallback | Low. Best-effort; may miss distro source-package naming |
| `BallerRegistry { url }` | none today — see [issue #10 section](#impact-of-issue-10--non-functional-sources) | n/a until the registry serves security data |

Reuse `parse_github_url` (`src/core/manifest.rs:353`) to turn a
`project_url`/`repository` string into an owner/repo pair — it already handles
`https://github.com/owner/repo`, `git@github.com:owner/repo.git`, and
`.../releases` forms.

### Chocolatey coverage (consequence #1)

Problem: OSV's NuGet ecosystem only carries advisories filed against
`nuget.org` ids; Chocolatey-only wrapper packages and the tools they install are
rarely covered by NuGet-scoped records. Combined with a Windows-only source,
Phase A would be weakest exactly on Windows.

Solution — the **derived identity**:

1. Query `(NuGet, pkg.name)` as today (catches advisory-bearing package ids).
2. Read `project_url` from the Chocolatey metadata (already parsed at
   `src/http/chocolatey.rs:46`). If it is a `github.com/owner/repo`,
   add `(GitHub, "{owner}/{repo}")` as a Derived identity and query it in the
   same batch.
3. Risk = max across both identities.

Rationale: the overwhelming majority of real CVEs for a tool like `7zip` are
filed against its upstream project, not its packaging id. Deriving the upstream
repo from metadata B.A.L.L.E.R. already downloads brings those records into the
Windows path with no new API surface. The same derivation applies to `repository`
on GitHub-sourced packages and `repository` on Cargo crates as an optional
future extension (both fields already exist on `Package`,
`src/core/package.rs:27`).

### OSV API client

New module `src/security/osv.rs` wrapping a single endpoint:

```
POST {osv_base}/v1/querybatch
Content-Type: application/json

{
  "queries": [
    { "package": { "ecosystem": "crates.io", "name": "serde" }, "version": "1.0.229" },
    { "package": { "ecosystem": "GitHub", "name": "BurntSushi/ripgrep" }, "version": "14.1.1" },
    ...
  ]
}
```

Response:

```
{ "results": [ { "vulns": [ { "id": "GHSA-xxxx-...", "modified": "...",
  "severity": [ { "type": "CVSS_V3", "score": "7.5" } ],
  "affected": [ { "ranges": [...], "versions": [...], "database_specific": {...} } ],
  "aliases": ["CVE-2023-XXXX"] } ] }, { "vulns": [] }, ... ] }
```

Design points:

- Only identities for packages that will actually be installed are queried.
- Non-advisory fields (summaries, references) are fetched from
  `GET /v1/vulns/{id}` lazily — only when a verdict needs human-readable
  reasoning, and only for the *narrowing* set of matched advisories.
- Host failure, 5xx, timeout, or malformed response → **fail-open**: each
  affected package is marked `Unverified` with a warning, and the install
  continues (config `fail_policy`, [Configuration](#configuration)).
- One batched request per command invocation. Cache lookup happens before the
  network call (see [Caching](#caching)).
- Use the existing `HttpClient` (`src/http/mod.rs`) so retries/timeouts behave
  identically to registry fetches.

### Version range matching

New module `src/security/ranges.rs`. OSV expresses affected versions as
`affected[].ranges` with `type: SEMVER | ECOSYSTEM | GIT` and `events` of
`introduced` / `fixed` / `last_affected`, plus optional explicit `versions`
lists. Referee:

- parses `ranges` + `versions` per affected entry into a normalised interval
  set;
- matches the package's version against it using B.A.L.L.E.R.'s existing
  `parse_version_flexible` (`src/core/dep_solver.rs`) which already strips
  Debian epochs/revisions, embedded tags, and normalises to semver-ish form
  (`2:8.1.0875-5ubuntu2` → `8.1.0875`, `1.21-76` → `1.21.0`);
- treats any matched interval on any affected entry as a hit for that advisory;
- prefers an explicit `versions` list when present (exact-match, no guessing).

`ranges.rs` is the most correctness-critical file in Referee. It must handle
half-open intervals (`introduced` only = "this version and later"), `fixed`
semantics (`>= introduced AND < fixed`), reversed event lists, and empty intro
(means "from the beginning"). Unit tests use real OSV fixtures (see
[Testing strategy](#testing-strategy)).

### Scoring and verdicts

OSV severity comes as CVSS. Referee converts the highest matched CVSS into a
**Referee Risk Index** on the user's 0–5 reference scale.

```
risk_index = highest_matched_cvss / 2.0        // 0.0..5.0
```

Two configurable thresholds split the index into three outcomes
([Configuration](#configuration)):

| Outcome | Condition (defaults) | Behaviour |
|---|---|---|
| `Pass` | `risk < warn_at` (default 2.5 → CVSS < 5.0, none/Low/Medium) | Silent install |
| `Warn` | `warn_at <= risk < block_at` (default 2.5–4.0 → CVSS 5.0–7.9, Medium/High) | Print per-advisory warning (aliases, CVSS, summary), install anyway; a prompt is only used when a *warned* package is also re-instocked and the user has not passed `--yes` |
| `Block` | `risk >= block_at` (default 4.0 → CVSS >= 8.0, High/Critical) | Abort the whole plan; every blocked package reported with aliases, score, and a one-line reason |

| Status (not severity) | Meaning |
|---|---|
| `Clean` | Queried, no advisory matched the version |
| `Vulnerable { risk }` | Advisory matched; scored and classified |
| `Unknown` | No ecosystem (BallerRegistry), lookup failed (fail-open), or advisory data unreadable |
| `Unverified` | OSV unreachable at check time; fail-open path |

Unknown ≠ safe. `Unknown`/`Unverified` packages are reported in a single summary
line at the end of the gate so the user can see what *wasn't* verified.

Verdict struct:

```rust
pub enum Verdict { Clean, Vulnerable { risk: f32 }, Unknown, Unverified }

pub struct AdvisoryVerdict {
    pub identity: AdvisoryIdentity,
    pub status: Verdict,
    pub matched: Vec<MatchedAdvisory>, // id, aliases, cvss, summary
}
```

---

## Phase B — the artifact scanner

Runs **after** download and extraction, **before** the binary is linked and
recorded — so a flagged artifact aborts with a clean rollback. The relevant
install path is `draft.rs:191-236` (`download_and_extract` → `create_symlink` →
`insert_package`); Referee Phase B slots between extraction and link, and on
failure cleans up through `Downloader::no_binary_error` (`downloader.rs:296`),
which removes the extract directory and cached archive — the same primitive the
`NoBinaryFound` path already uses.

Scope by source:

| Source | Phase B? | Why |
|---|---|---|
| GitHub, BallerRegistry, Chocolatey | Yes | B.A.L.L.E.R. downloads and extracts the archive |
| System, Cargo | No | Content is installed by the native tooling; no artifact passes through B.A.L.L.E.R. |

### Offline heuristics

Run against every regular file in the extracted tree, with per-file text/binary
classification mirroring `find_binary_in_dir` (`src/utils/fs.rs:117-142`) so the
scanner and the binary-finder agree on what a "file that runs" is.

1. **Suspicious embedded command strings** in script files and (via a
   `strings`-style decode pass) plain binaries:
   - base64/hex-decoded payload piped into a shell (`echo <b64> | base64 -d | sh`,
     PowerShell `[Convert]::FromBase64String` + `iex`);
   - `curl ... | sh` / `Invoke-WebRequest ... | iex` download-execute chains;
   - writes to `/tmp` (or `%TEMP%`) followed by an exec/start of the written file;
   - exfil patterns: reads of `AWS_*`, `AZURE_*`, `SECRET`, `TOKEN`, `PGPASSWORD`
     env-var names combined with an external send (`curl`, `nc`, `Invoke-WebRequest`).
2. **Unexpected executable / startup files**:
   - an executable outside the set that will actually be linked, especially
     `.desktop` launcher entries, `run.reg`, `.lnk` shortcuts, autorun/startup
     filenames, or installers (`.msi`, `.ps1` installers) bundled inside the archive;
   - script files that take no arguments and run at "priority" statements (e.g., a
     `.vim` autoload, profile/rc files injected into `$HOME`).
3. **Obfuscation signals**: unusually high Shannon entropy in a *small* script
   file (a few-KB script that reads as random data is an execution-time bomb
   pattern).
4. **Unsafe permissions** (Unix only, see below).
5. Keep the signal list compact, documented, and unit-tested. A verdict is the
   **union** of one or more hits:

```rust
pub struct ScanFinding {
    pub path: PathBuf,
    pub rule: ScanRule,   // SuspiciousPayload, ExfilAttempt, UnexpectedExecutable, HighEntropy, UnsafePermissions
    pub evidence: String, // truncated matching snippet or computed value
}
```

### Linux-specific checks

- **Permissions via `std::os::unix::fs::PermissionsExt`**: any extracted file
  with setuid/setgid bits, or world-writable (`mode & 0o7777`), is flagged
  (`UnsafePermissions`).
- Executable identity is the **exec bit** (`utils/fs.rs:131`) — Linux binaries
  are matched by mode, so the scanner uses the same definition.
- Script dialects: `.sh`, `.py`, `.pl`, `.bash`. Payload patterns target
  `curl|sh` (dash, bash, zsh, sh), `/tmp` write-then-exec, `system(`/`exec(` in
  scripts.
- Cached archives live under `~/.baller/cache/`; on a block the `no_binary_error`
  cleanup is used.

### Windows-specific checks

- **No setuid concept** — that rule is `#[cfg(unix)]`-gated and skipped.
  Permission check replaced by flagging unusual-extension runnables.
- Executable identity is **extension**: `.exe`, `.dll`, `.bat`, `.cmd`,
  `.ps1`, `.vbs`, `.js` (`utils/fs.rs:138`).
- Script dialects: PowerShell / batch. Payload patterns target `iex`,
  `Invoke-Expression`, `[Convert]::FromBase64String`, `Invoke-WebRequest
  -OutFile`, `reg add HKCU\...\Run` persistence, scheduled-task creation,
  `certutil.exe -urlcache` download-and-decode chains.
- `fs::copy` installs (`windows.rs:59`) mean the *linked* artifact is a copy —
  scanning the extracted tree before the copy is equivalent to scanning what
  ends up in `%LOCALAPPDATA%\baller\bin`.

### Optional VirusTotal hook

Behind config `referee.virustotal_api_key`. When set, Phase B additionally
submits SHA-256/imp hashes of the linkable binary (and any other PE/ELF
executables found) to the VirusTotal file-report endpoint. Deliberately **hash
only** — never the file contents — to avoid exfiltrating a user's binaries to a
third party. Timeout + API-key absence both degrade to "no VT result", and do
not change the verdict from the offline scan. This is staged in
[Milestone 3](#milestones).

---

## Integration points

### New service in `AppContext`

`AppContext` (`src/context.rs:45`) gains a `referee: Referee` field, built in
`AppContext::new` from `config.referee` and the existing `HttpClient` — the same
pattern as `Downloader`/`RegistryClient`. `Referee` holds:

```rust
pub struct Referee {
    osv_client: OsvClient,
    thresholds: RefereeThresholds, // warn_at, block_at
    fail_policy: FailPolicy,
    db_cache: RefereeCache,        // SQLite-backed
    scanner: ArtifactScanner,
}
```

### Command hooks

| Location | Insertion |
|---|---|
| `src/commands/draft.rs` | Phase A gate after `resolve_deps_with_root` (`draft.rs:64`), before `report_plan`/install loop. Phase B after `download_and_extract` (`draft.rs:191`), before `create_symlink` (`draft.rs:216`) |
| `src/commands/update.rs` | Phase A on `remote_pkg` (the version being installed). New-install path (`update.rs:118-161`) and upgrade path (`update.rs:184-213`) get Phase B after their `download_and_extract` |
| `src/commands/substitute.rs` | Phase A on the replacement package, before ejecting the old one; Phase B on the replacement's artifact |

Phase A results for every checked package are cached before the loop; no
duplicate network work across `draft`/`update`/`substitute`.

### New subcommand: `baller referee`

- `baller referee` — audit every installed package from the DB
  (`DbManager::list_packages`): Phase A query for each, Phase B re-scan of
  `bin_path`s still on disk. Table output; `--json` supported; warn/block do
  **not** modify the install (audit is read-only).
- `baller referee <package>` — audit a single package by name (or
  `owner/repo` / `crate` forms).

### Bypass flag

Global `--no-referee` on `BallerCommand` (`src/cli/parse.rs`), carried through
`GlobalFlags` (`src/context.rs:28`) like `--yes`/`--quiet`/`--json`. Disables
both phases. Never present when `referee.enabled = false` in config; the flag
is the CLI escape hatch and config is the policy default.

### Hook ordering

Referee runs **before** `run_hook(PreInstall, …)` (`draft.rs:101`) so a blocked
package never fires install hooks — a hook running against a package Referee
rejected would be as bad as the block itself. PostInstall hooks run unchanged.

---

## Configuration

New `RefereeConfig` in `src/config/config.rs`, parsed under a `[referee]`
section in `baller.conf`, following the existing key/value parser pattern
(`config.rs:154`). Unknown-section errors already exist (`config.rs:240`), so
`[referee]` must be added there.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `true` | Master switch for both phases |
| `warn_at` | float (0–5) | `2.5` | Values ≥ this warn (CVSS ≥ 5.0) |
| `block_at` | float (0–5) | `4.0` | Values ≥ this block (CVSS ≥ 8.0) |
| `fail_policy` | `fail-open` \| `fail-closed` | `fail-open` | What to do when OSV is unreachable |
| `osv_base_url` | string | `https://api.osv.dev` | For tests/self-hosting |
| `virustotal_api_key` | string | unset | Enables the [VirusTotal hook](#optional-virustotal-hook) |

Validation: `0 <= warn_at < block_at <= 5`. `warn_at == 0` warns on anything;
`block_at == 0` blocks anything with an advisory at all.

---

## Caching

SQLite table in the existing DB (`DbManager`, `~/.baller/db/baller.db`) so audit
re-runs and repeated installs are offline-fast and no op-against-OSV happens at
`update` for packages already vetted this session.

```sql
CREATE TABLE IF NOT EXISTS referee_cache (
  ecosystem   TEXT NOT NULL,
  name        TEXT NOT NULL,
  version     TEXT NOT NULL,
  verdict     TEXT NOT NULL,        -- Clean|Vulnerable|Unknown|Unverified
  risk        REAL,
  advisories  TEXT NOT NULL,        -- JSON: matched advisory summaries
  checked_at  TEXT NOT NULL,
  PRIMARY KEY (ecosystem, name, version)
);
```

- Phase A consults the cache first; OSV is only hit on a miss.
- Cache entries are **verdicts**, and a `Clean` verdict is only trusted for the
  exact `(ecosystem, name, version)` it was computed from.
- `baller sweep` semantics: cache rows are small and safe to leave, but the
  `referee` audit command gains `--refresh` to force re-query.
- `fail-closed` policy treats a cache miss + network failure as a block.

---

## Errors

New variants in `src/error/error.rs`, threaded through `BallError`:

```rust
RefereeBlocked {
    package: String,
    version: String,
    advisories: Vec<AdvisoryRef>,   // id, aliases, cvss, short summary
    reason: String,
},
RefereeScanBlocked {
    package: String,
    findings: Vec<ScanFinding>,
},
RefereeUnavailable { message: String },  // raised as an *informational* warn under fail-open; fatal only under fail-closed
```

Exit codes and message styling follow existing conventions (colored, hook-shaped
`tracing` output). Blocked installs return non-zero like any other failed
command.

---

## JSON output

Honour `ctx.flags.json` wherever Referee prints. Phase A gate emits a compact
array; the `referee` command emits a full audit table:

```json
{
  "command": "referee",
  "packages": [
    {
      "name": "ripgrep",
      "version": "14.1.1",
      "source": "github:BurntSushi/ripgrep",
      "status": "vulnerable",
      "risk": 3.75,
      "advisories": [
        { "id": "GHSA-xxxx", "aliases": ["CVE-2023-XXXX"], "cvss": 7.5 }
      ]
    }
  ]
}
```

---

## Impact of issue #10 — non-functional sources

Issue #10 (https://github.com/HMythical/baller/issues/10) documents that the
Baller Registry client is fully wired (`src/http/registry_api.rs`,
`RegistrySource::BallerRegistry`) but the **server behind
`https://registry.baller.dev/api` does not exist** (NXDOMAIN). Every default
lookup fires a doomed `baller`-first request, retries with backoff, and falls
through to real sources. Referee must be designed against this reality and
against the related gap for GitHub-distributed "baller" packages that carry no
B.A.L.L.E.R.-native metadata.

Referee's stance:

1. **Functionally unaffected.** Referee keys off the *resolved*
   `pkg.source`, which is whatever actually answered. A package can only be
   checked if it resolved; the failing `baller` head consumes time (and the
   issue #87 fix will remove it from the default chain) but produces no
   security decision.
2. **A `BallerRegistry`-resolved package has no advisory identity** *today* and
   is reported `Unknown` (fail-open). Referee must not invent an ecosystem for
   it — a wrong guess is worse than a truthful "unknown".
3. **Native registry advisory data (future, unblocks #10).** When the registry
   server exists, its package metadata should carry security data, and Referee's
   `registry_identities` gains two paths:

   a. **Self-declared OSV identity.** `baller build` manifests gain an optional
      `[advisory]` section (`ecosystem`, `name`, plus optional `aliases`),
      stamped into the package metadata the registry serves. Authors declare
      where their known-issue surface lives (e.g. a crate also published on
      crates.io declares `ecosystem = "crates.io"` so RustSec records apply).
      This turns `Declared` identities into first-class coverage for packages
      whose distribution shape OSV cannot map.

   b. **Registry-native advisories.** The registry API grows an optional
      `vulnerabilities` field on package metadata (an OSV-*shaped*
      `vulns` array). When present, Referee consumes it as an authoritative
      identity (`IdentityScope::Primary`) and skips guessing.

4. **GitHub-sourced packages** without baller-native metadata rely on the
   `(GitHub, owner/repo)` identity — the same record set GHSA advisories use,
   available today. This is the strongest cross-OS surface B.A.L.L.E.R. has and
   the spine of the Phase A story.

These are design commitments, not deliverables: Milestone 3 tracks the
manifest `[advisory]` schema; the registry-server work itself is issue #10's
scope.

---

## Runtime behaviour on Linux and Windows

### Shared plumbing

Phase A gates the whole plan before the loop; Phase B runs per artifact before
it is linked. Both use identical `Referee` code; only the identity expansion,
scan rules, and executable classification differ per OS.

### Linux

Defaults (`config.rs`, `system.rs`): chain `baller → system → cargo → github`;
manager from `/etc/os-release`; installs into `~/.local/bin` via symlink.

`baller draft vim` → `System{apt}`, version `2:8.1.0875-5ubuntu2`:

1. Phase A maps to `(Debian, vim)` — Fallback, best-effort. `parse_version_flexible`
   yields `8.1.0875` for range matching.
2. Block → abort before `sudo apt-get install` is spawned; zero writes.
3. Warn → print advisory line; continue into `sudo`.
4. Unknown → fail-open, continue.
5. **No Phase B** (native channel installed the content).

`baller draft ripgrep` → `GitHub{BurntSushi/ripgrep}`, `14.1.1`:

1. `github.rs:227` picks the `x86_64-unknown-linux-gnu` asset (host-specific),
   so the version checked is the Linux build.
2. Phase A: `(GitHub, BurntSushi/ripgrep)` — repo-scoped advisories.
3. Download → SHA-256 verify → extract → `find_binary_in_dir` (exec bit).
4. Phase B scans the tree: ELF binary + any `.sh`/`.py`/`.pl`, Unix permission
   checks (`mode & 0o7777`), `curl|sh`/`/tmp` payload patterns, extra
   executables.
5. Scan flags → abort before `LinuxManager::create_symlink`; `no_binary_error`
   cleanup removes extract dir + cached archive.
6. Clean → symlink into `~/.local/bin`, DB record, PostInstall hook.

`baller draft serde` → `Cargo{crate_name}`: Phase A `(crates.io, serde)` —
the cleanest mapping (semver + RustSec). No Phase B.

### Windows

Defaults: chain `baller → chocolatey → github`; system and cargo sources
disabled (`system_enabled`/`cargo_enabled` = `cfg!(linux)`); installs are
`fs::copy` into `%LOCALAPPDATA%\baller\bin` with an `.exe` suffix
(`windows.rs:35-61`).

`baller draft 7zip` → `Chocolatey`, `19.00`:

1. Phase A expands to **two identities**: `(NuGet, 7zip)` Primary and
   `(GitHub, derived-owner/repo)` Derived from `project_url`. Risk = max.
2. Block → abort before the `.nupkg` fetch.
3. Pass → download `.nupkg`; Chocolatey sets `hash_algorithm="SHA512"`, so the
   base64→SHA-512 verify path runs (`downloader.rs:63-76`); extract as zip.
4. Phase B: PE + `.exe`/`.dll`/`.bat`/`.cmd`/`.ps1` classification; PowerShell
   `iex`/`[Convert]::FromBase64String`/`reg add` patterns; no setuid checks
   (`#[cfg(unix)]` gated); unexpected installers/.lnk/autorun files flagged.
5. Scan flags → abort before the `fs::copy`; cleanup as on Linux.
6. Clean → copy to `...\bin`, DB record, PostInstall hook.

`baller draft ripgrep` on Windows resolves the same repo but selects the
`x86_64-pc-windows-msvc.zip` asset — advisory identity is OS-agnostic, the
artifact and version checked are Windows-specific.

### Notable deltas

| Concern | Linux | Windows |
|---|---|---|
| Verdict-critical sources | system, cargo, github | chocolatey, github |
| No-archive sources (Phase A only) | system, cargo | — |
| Version normalisation | epochs/revisions → `parse_version_flexible` | NuGet 4-part→3-part (already done) + `parse_version_flexible` |
| Binary identity | exec bit | `.exe`/`.bat`/`.cmd` extension |
| Permission red flags | setuid/setgid/world-writable | n/a |
| Script dialects scanned | `.sh`/`.py`/`.pl` | `.ps1`/`.bat`/`.cmd`/`.vbs`/`.js` |
| Install step after scan | symlink (unprivileged) | `fs::copy` `.exe` |
| System source | apt/dnf/pacman via sudo | unavailable |
| `baller referee` re-scan | symlinked binaries + system/cargo DB rows | copied `.exe` in `...\bin` |

---

## Testing strategy

### Unit tests

- `security/ranges.rs` — OSV range fixtures (JSON captured from real OSV APIs):
  half-open `introduced`-only, `introduced`+`fixed`, `last_affected`, explicit
  `versions` lists, reversed events, ECOSYSTEM vs SEMVER range types; matching
  flexible forms (`2:1.21-76`, `8.2.2637-20.fc36`, `14.1.0.0`).
- `security/scoring.rs` — CVSS→Risk-Index conversion, threshold banding
  (pass/warn/block), `warn_at == 0`, `block_at == 0`, invalid config rejection.
- `advisory_identities` — every `PackageSource` variant → expected identity set,
  including Chocolatey `project_url` derivation (`github.com` and non-GitHub
  URLs) and BallerRegistry → empty/Unknown.
- `security/scan.rs` — synthetic tree fixtures: benign binary + benign script
  ⇒ no findings; each rule fires against a purpose-built malicious fixture
  (base64→sh, `curl|sh`, `iex` payloads, setuid file under `#[cfg(unix)]`,
  autorun `.lnk`, high-entropy mini-script). Assert abbreviated `evidence`.
- `config.rs` — `[referee]` parse, defaults, validation bounds, unknown-section
  behaviour preserved.
- `baller referee` JSON shape and audit of an empty DB.

### Integration tests

- Spin a local mock OSV server (`osv_base_url` points at it) returning fixture
  batches: unknown package → continue; Medium advisory → warn + install;
  Critical advisory → `RefereeBlocked`, zero DB rows, zero symlinks.
- Fail-open: mock server returns 500/connection-refused → packages
  `Unverified`, install still writes.
- Fail-closed: same outage → install aborted.
- Blocked dependency inside a 3-package plan → **nothing** installed (validates
  the pre-loop gate; the regression this exists to prevent is partial installs).
- Phase B: malicious zip fixture → `RefereeScanBlocked`, extract dir and cached
  archive removed, no roster entry.
- `--no-referee` bypass skips both phases entirely.

Run via the existing `./build/linux/build.sh test` / PowerShell `build.ps1 -Command test` pipelines.

### Benchmarks

`src/benches/workflow.rs` gains a referee pass measuring batch-query latency
against the mock server and cache-hit performance.

---

## Milestones

### Milestone 1 — Advisory gate (Phase A)

- `src/security/{mod,osv,ranges,scoring,verdict.rs}` — data types, OSV client,
  range matching, risk index, verdicts.
- `advisory_identities` expansion incl. Chocolatey derived-GitHub identity.
- `Referee` service + `AppContext` wiring; `RefereeConfig` + `[referee]` parse.
- Phase A pre-loop gate in `draft.rs`, `update.rs`, `substitute.rs`.
- DB cache table; `referee` audit command; `--no-referee` flag; `BallError`
  variants; JSON output.
- Issue #10 stance implemented: BallerRegistry → Unknown; no invented
  ecosystems; derived identity from `project_url` only.

### Milestone 2 — Artifact scanner (Phase B)

- `src/security/scan.rs` — rule engine, per-OS dialects + classification,
  `#[cfg(unix)]` permission rules.
- Phase B slots into `draft`/`update`/`substitute` after extraction, before
  linking; `no_binary_error` cleanup on flag; `RefereeScanBlocked`.
- Scan fixtures + tests; bench pass.

### Milestone 3 — Registry-native security data & polish

- `baller build` manifest `[advisory]` section (self-declared OSV identity),
  surfaced through `registry_identities` as `Declared`.
- Optional VirusTotal hash hook behind `virustotal_api_key`.
- `docs/referee.md` write-up; README source table updates; CHANGELOG entries
  per milestone; full integration suite on both platforms.

---

## Open questions

1. **Chocolatey id ↔ NuGet id drift**: when a Chocolatey-only package's id
   matches no NuGet record *and* `project_url` is empty/non-GitHub, the package
   is `Unknown`. Acceptable, or should Phase A also query NVD by name as a
   Fallback identity at the cost of a second API surface?
2. **Warn + prompt**: should a `Warn`-band package on an *interactive* terminal
   prompt for confirmation (like `eject` does) rather than just printing?
   Recommended default: print-only; a prompt would interrupt scripted installs
   that already passed `--yes`.
3. **`referee --refresh` vs TTL**: cache rows are currently permanent until
   rewritten. Do audit runs want a configurable TTL (e.g. re-check after
   24h) instead of only `--refresh`?
4. **Distro ecosystems**: OSV carries `Debian`/`Fedora`/`AlmaLinux` etc. but
   distro *version strings* only align with `parse_version_flexible`
   heuristically. Confirm the Fallback scope (apt/dnf only, pacman → Unknown)
   before Milestone 1 closes.