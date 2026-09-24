# Referee — Package Security

Referee is B.A.L.L.E.R.'s package security layer. It checks every package a
command installs — the requested package *and* every resolved dependency —
against public vulnerability data, and inspects each downloaded archive before
its binary is linked.

It is enabled by default. `--no-referee` turns it off for one command;
`enabled = false` under `[referee]` in `baller.conf` turns it off permanently.

---

## Contents

1. [The two phases](#the-two-phases)
2. [Advisory identities](#advisory-identities)
3. [Verdicts and the risk index](#verdicts-and-the-risk-index)
4. [Phase A — the advisory gate](#phase-a--the-advisory-gate)
5. [Phase B — the artifact scan](#phase-b--the-artifact-scan)
6. [`baller referee` — the command group](#baller-referee--the-command-group)
7. [Export formats](#export-formats)
8. [Configuration](#configuration)
9. [Declaring an advisory identity](#declaring-an-advisory-identity)
10. [Caching](#caching)
11. [JSON output](#json-output)
12. [Errors](#errors)
13. [Code map](#code-map)
14. [Scope and limits](#scope-and-limits)

---

## The two phases

| Phase | Runs | Scope | Can block? |
|---|---|---|---|
| **A — advisory gate** | Before the install loop, on the whole resolved plan | Every package, root and dependencies | Yes — aborts before anything is written |
| **B — artifact scan** | After download and extraction, before the binary is linked | Sources that download archives: GitHub, Baller registry, Chocolatey | Yes — aborts and purges the download |

System and Cargo packages produce no artifact B.A.L.L.E.R. handles — `apt` and
`cargo install` fetch and place their own content — so only Phase A applies to
them.

Phase A runs before the `pre_install` hook, so a blocked package never executes
a hook of its own.

Where each phase sits per command:

| Command | Phase A | Phase B |
|---|---|---|
| `draft` | On the full resolved plan, before the install loop | After extraction, before `create_symlink` |
| `update` | On the new version and any dependencies it pulls in | After extraction, **before** the old extract directory is pruned |
| `substitute` | On the replacement and its dependencies, before the old package is ejected | After extraction, before `create_symlink` |

Because Phase A gates the whole plan up front, a block writes nothing at all —
no symlink, no roster row, no cached archive — regardless of where in the plan
the offending package sits.

Because `update` scans before pruning, a rejected upgrade leaves the previously
installed version linked and runnable.

---

## Advisory identities

A package is expanded into every identity public advisory data may know it by.
All of them are queried in one batched request, and the **highest** risk found
under any of them becomes the package's risk.

| Source | Identity | Scope |
|---|---|---|
| `Cargo { crate_name }` | `(crates.io, crate_name)` | `Primary` |
| `GitHub { owner, repo }` | `(GitHub, owner/repo)` | `Primary` |
| `Chocolatey` | `(NuGet, name)` | `Primary` |
| `Chocolatey` whose `project_url` points at `github.com/owner/repo` | `(GitHub, owner/repo)` | `Derived` |
| `System { apt }` | `(Debian, name)` | `Fallback` |
| `System { dnf }` | `(Fedora, name)` | `Fallback` |
| `System { pacman }` | none — reported `unknown` | — |
| `BallerRegistry` | none, unless declared | — |
| Any source with an `[advisory]` section | the declared `(ecosystem, name)` | `Declared` |
| Any source whose metadata carries `vulnerabilities` | the registry's own feed | `Primary`, no network call |

`IdentityScope` records where an identity came from, and appears in
`--json` output as `scope`:

| Scope | Source of the identity |
|---|---|
| `primary` | The package source's own ecosystem |
| `derived` | Read out of package metadata, such as a Chocolatey `project_url` |
| `declared` | Stated by the package itself, through `[advisory]` |
| `fallback` | A best-effort distro mapping |

Identities naming the same `(ecosystem, name)` are collapsed, keeping the
strongest scope (`declared` > `primary` > `derived` > `fallback`).

The derived Chocolatey identity exists because NuGet advisories are filed
against `nuget.org` ids: a Chocolatey wrapper for a tool like 7-Zip usually has
no NuGet record, while the tool's own project does. Referee reads the
`project_url` that already arrives with the Chocolatey metadata
(`src/http/chocolatey.rs`), turns it into an `owner/repo` pair with
`parse_github_url` (`src/core/manifest.rs`), and queries both identities in the
same batch.

An ecosystem is never inferred from a package name. A package that maps to no
identity is reported `unknown`.

---

## Verdicts and the risk index

Each identity produces one of four verdicts, and a package's overall verdict is
the worst of its identities':

| Verdict | Meaning |
|---|---|
| `clean` | Queried, and no advisory matched this version |
| `vulnerable` | An advisory matched this version |
| `unknown` | No identity to query, or the data could not be read |
| `unverified` | The advisory service was unreachable |

Aggregation rules, in order:

1. Any `vulnerable` identity makes the package `vulnerable`, at the highest risk
   found.
2. Otherwise, any `unverified` identity makes the package `unverified`.
3. Otherwise, if *every* identity is `unknown`, the package is `unknown`.
4. Otherwise the package is `clean` — at least one identity answered.

`unknown` and `unverified` are reported, never presented as safety. Both appear
in a closing summary line naming what was not verified.

### Scoring

Severity arrives from OSV as a CVSS vector string. Referee computes the base
score — CVSS v3.0/v3.1 and v2 are implemented from their published formulas, and
a bare number is taken as already scored — then halves it onto a 0–5 **Referee
Risk Index**:

```
risk_index = highest_matched_cvss / 2.0
```

Scores are read in this order, first hit wins:

1. the advisory's own `severity` list;
2. the `severity` on a matching `affected` entry;
3. `database_specific.severity`, mapped `CRITICAL` → 9.0, `HIGH` → 7.5,
   `MODERATE`/`MEDIUM` → 5.0, `LOW` → 3.0.

CVSS v4.0 vectors are not computed from the vector; such an advisory falls
through to step 3.

### Banding

| Band | Default condition | Behaviour |
|---|---|---|
| `pass` | `risk < warn_at` (2.5, i.e. CVSS < 5.0) | Install silently |
| `warn` | `warn_at <= risk < block_at` (CVSS 5.0–7.9) | Print the advisory, install anyway |
| `block` | `risk >= block_at` (4.0, i.e. CVSS >= 8.0) | Abort the whole plan |

A matched advisory with **no** published severity bands as `warn`: it cannot be
scored, so it is never passed as safe and never blocks. Only a `vulnerable`
package is banded — an `unknown` or `unverified` package is reported, never
blocked, however strict the thresholds.

### Version matching

An advisory's `affected` entries are matched locally as well as by the service:

- An explicit `versions` list is matched first, by exact string and then by
  normalised comparison, so a record listing `1.21-76` matches an installed
  `1.21`.
- `ranges` are normalised into intervals from their `introduced` / `fixed` /
  `last_affected` / `limit` events. `fixed` excludes its own version,
  `last_affected` includes it, an `introduced` with no terminator runs to
  infinity, and a terminator with no `introduced` starts from zero. Events are
  sorted before they are walked, so out-of-order event lists behave the same as
  sorted ones.
- `SEMVER` and `ECOSYSTEM` ranges are both evaluated through
  `parse_version_flexible` (`src/core/dep_solver.rs`), which normalises Debian
  epochs, revisions, Fedora tags and zero-padded segments —
  `2:8.1.0875-5ubuntu2` becomes `8.1.875`. `GIT` ranges carry commits rather
  than versions and are skipped.
- Ecosystems compare on the part before `:`, so OSV's `Debian:11` matches
  `Debian`.
- If none of a record's `affected` entries name the identity being checked, the
  service's own filtering is kept rather than the record being discarded.

---

## Phase A — the advisory gate

A blocked plan:

```
$ baller draft badtool
Drafting badtool...

[Error]: referee blocked 1 package(s); nothing was installed
	badtool v2.0.0 — risk index 4.90 is at or above the block threshold of 4.00
	  • GHSA-crit (CVE-2026-0001) CVSS 9.8 — badtool is affected
	run with --no-referee to install anyway, or raise referee.block_at
```

A blocked dependency stops the whole plan, and nothing is downloaded:

```
$ baller draft root          # root -> middle -> leaf; middle is critical
[Error]: referee blocked 1 package(s); nothing was installed
	middle v0.4.0 — risk index 4.90 is at or above the block threshold of 4.00

$ baller roster
Empty No packages installed. Use 'baller draft <name>' to install.
```

A warning prints and continues:

```
$ baller draft warntool
Drafting warntool...
Referee warntool v3.0.0: risk index 3.05 of 5.00
    • GHSA-med (CVE-2026-0002) CVSS 6.1 — warntool is affected
Done warntool v3.0.0 drafted!
```

`--dry-run` reports a block instead of raising it, and exits 0:

```
$ baller draft badtool --dry-run
Dry run badtool v2.0.0 from baller
  • badtool v2.0.0 (root, install)
    Download: https://example.test/badtool.tar.gz
  ✗ badtool v2.0.0 would be blocked — risk index 4.90 is at or above the block threshold of 4.00
    • GHSA-crit (CVE-2026-0001) CVSS 9.8 — badtool is affected
Note nothing was installed
```

A rejected upgrade leaves the working version in place:

```
$ baller update upd
Updating upd: 1.0.0 -> 2.0.0
[Error]: referee blocked the downloaded archive for 'upd' v2.0.0 — the download was discarded
	• block [suspicious-payload] hook.sh — /dev/tcp/10.0.0.9/4444

$ baller roster
  • upd v1.0.0
$ upd
hello from upd
```

### Requests made

One `POST {osv_base_url}/v1/querybatch` per 100 identities, plus one
`GET /v1/vulns/{id}` per *distinct* advisory that matched and needs its
severity, summary and ranges. A clean plan costs exactly one request; an
advisory affecting five packages in one plan is fetched once. Identities
answered from the cache cost nothing.

A detail fetch that fails does not discard the finding: the advisory is still
reported, with no score, which bands as `warn`.

Withdrawn advisories are ignored.

### When the advisory service is unreachable

`fail_policy = fail-open` (the default) reports the affected packages as
`unverified` and continues:

```
$ baller draft tool
Referee 1 package(s) were not verified: tool (unverified)
Done tool v1.0.0 drafted!
```

`fail_policy = fail-closed` refuses instead:

```
[Error]: referee could not verify this install: network error: HTTP 500 …
         (fail_policy = fail-closed, so nothing was installed)
```

An `unverified` verdict is never cached.

---

## Phase B — the artifact scan

Every regular file in the extracted tree is read: text files directly, binaries
through a `strings`-style pass, so an embedded address or command is still
found. Matches are reported at most once per rule per file.

| Rule | Matches | Severity |
|---|---|---|
| `suspicious-payload` | `base64 -d \| sh`, `echo <blob> \| base64`, `[Convert]::FromBase64String` with `iex`, `certutil -urlcache`, `iex(New-Object Net.WebClient)`, `DownloadString(...) \| iex`, `powershell -enc`, `-ExecutionPolicy bypass` with `-WindowStyle hidden` | **block** |
| `suspicious-payload` | `/dev/tcp/host/port`, `nc -e /bin/sh` | **block** |
| `suspicious-payload` | `reg add …\CurrentVersion\Run` | **block** |
| `suspicious-payload` | `chmod +x /tmp/…`, `%TEMP%\…` followed by `Start-Process` | **block** |
| `suspicious-payload` | appending to `$HOME/.bashrc`, `.zshrc`, `.profile`, `.bash_profile` | **block** |
| `suspicious-payload` | `curl … \| sh`, `iwr … \| iex`, `schtasks /create`, `New-ScheduledTask*`, `crontab -`, `/etc/cron.d/`, `systemctl enable` | warn |
| `exfil-attempt` | A credential env-var name (`AWS_SECRET_ACCESS_KEY`, `AZURE_CLIENT_SECRET`, `GITHUB_TOKEN`, `NPM_TOKEN`, `PGPASSWORD`, `OPENAI_API_KEY`, …) within 240 characters of `curl`/`wget`/`Invoke-WebRequest`/`nc`/`/dev/tcp/`/`WebClient`, in either order | **block** |
| `exfil-attempt` | A read of `~/.ssh/id_*`, `.aws/credentials`, `.config/gcloud/credentials`, `.npmrc` or `.docker/config.json` followed by a send | **block** |
| `unsafe-permissions` | setuid or setgid bits (`mode & 0o6000`), Unix only | **block** |
| `unsafe-permissions` | world-writable (`mode & 0o002`), Unix only | warn |
| `unexpected-executable` | `.lnk`, `.url`, `.scr`, `.pif`, `.hta`, `.jse`, `.wsf`, `.wsh`, `.msi`, `.msp`, `.reg`, `.desktop`, `.cpl`, `.appref-ms`, and the filenames `autorun.inf`, `desktop.ini`, `.bashrc`, `.zshrc`, `.profile`, `.bash_profile` | warn |
| `high-entropy` | A script of 64 B–8 KB whose Shannon entropy exceeds 5.8 bits/byte | warn |
| `virustotal-detection` | At least one engine flags the file's SHA-256 (only with `virustotal_api_key`) | **block** |

All patterns are matched case-insensitively, and proximity patterns use bounded
gaps, so a match means the halves of a behaviour appear together rather than
merely in the same file.

A **block**-severity finding aborts the install; **warn**-severity findings are
printed and the install continues:

```
$ baller draft noisy
Referee noisy v1.0.0: suspicious-payload in bootstrap.sh — curl -fsSL https://example.test/x | sh
Done noisy v1.0.0 drafted!
```

### Per-OS behaviour

| Concern | Linux | Windows |
|---|---|---|
| "Executable" means | the exec bit | `.exe` / `.bat` / `.cmd` |
| Permission rules | setuid/setgid, world-writable | not applicable (`#[cfg(unix)]`-gated) |
| Script dialects read as text | `.sh` `.bash` `.zsh` `.ksh` `.py` `.pl` `.rb` `.lua` `.fish` `.awk` `.tcl` `.nu` `.r` `.php` | `.ps1` `.psm1` `.bat` `.cmd` `.vbs` `.js` `.mjs` `.cjs` |
| Install step the scan precedes | `symlink` into `~/.local/bin` | `fs::copy` into `%LOCALAPPDATA%\baller\bin` |

The executable definition matches `find_binary_in_dir` (`src/utils/fs.rs`), so
the scanner and the binary finder agree on what a program is.

### Read limits

| Limit | Value |
|---|---|
| Bytes read per file | 8 MB |
| Bytes read per tree | 256 MB |
| Files opened per tree | 20 000 |
| Directory depth | 24 |
| Minimum printable run pulled from a binary | 6 characters |
| Evidence quoted per finding | 120 characters |

Symlinks are not followed: a symlink either points inside the tree, which is
already walked, or outside it, which is not the package's content. When a limit
is reached the scan stops and says so at `--verbose`.

### On a block

The extract directory and the cached archive are both removed through
`Downloader::purge_download` (`src/core/downloader.rs`), the same cleanup
`NoBinaryFound` performs, so a retry re-downloads rather than reusing a rejected
archive.

### VirusTotal

Setting `virustotal_api_key` adds a hash lookup. Executables are found with the
same definition the rest of Phase B uses — the exec bit on Linux, `.exe` /
`.bat` / `.cmd` on Windows — and for each one a single
`GET {virustotal_base_url}/files/{sha256}` is made with the key in an `x-apikey`
header. Only the digest is sent; the request has no body and file contents never
leave the machine. At most eight executables per artifact are looked up.

A report with `last_analysis_stats.malicious > 0` becomes a block-severity
`virustotal-detection` finding. A missing key, a timeout, a rate limit, a
network failure, a 404 (an unrecognised hash) and a clean report all produce no
finding, and the offline scan's result stands unchanged.

`virustotal_base_url` points the lookup elsewhere, which is what makes the hook
testable and usable behind a self-hosted proxy.

---

## `baller referee` — the command group

```
baller referee [PACKAGE_NAME] [--refresh] [--no-scan]           # ≡ audit
baller referee audit  [PACKAGE...] [--refresh] [--no-scan]
                      [--fail-on block|warn] [--format json|markdown|sarif] [--out FILE]
baller referee check  [PACKAGE...] [--refresh] [--fail-on block|warn]
baller referee scan   [PACKAGE...]
baller referee cache  [--status | --clear | --prune <DAYS>]
baller referee config
baller referee sbom   [--out FILE] [--format cyclonedx-json]
```

Every subcommand is read-only with respect to the roster: nothing is ejected,
updated, linked or unlinked. `cache` writes only to the verdict cache. `audit`,
`check` and `scan` refuse to run with Referee disabled; `cache`, `config` and
`sbom` do not consult it and work either way.

A package name is resolved against the roster in three forms — the exact name,
the part after the last `/`, and the part after the last `:` — so `ripgrep`,
`BurntSushi/ripgrep` and `cargo:ripgrep` all find the same row. `audit`,
`check` and `scan` accept several names; a name given twice is audited once.

### `audit` — Phase A and Phase B

The bare `baller referee [PACKAGE_NAME]` runs `audit` with unchanged behavior.
It rebuilds each installed package from its roster row — including the advisory
identity it was installed under — checks it against advisory data, and re-scans
its extracted tree where one is still on disk.

```
$ baller referee --refresh
Refereeing 2 package(s) — warn at 2.50, block at 4.00 on the 0-5 risk scale
PACKAGE                  VERSION        STATUS       RISK     ADVISORIES
alpha                    1.0.0          vulnerable   4.90     GHSA-late
    • GHSA-late (CVE-2026-4242) CVSS 9.8 — a serious flaw in alpha
beta                     2.0.0          clean        —        —
    ! warn [suspicious-payload] bootstrap.sh — curl -fsSL https://x.test | sh

Summary 2 package(s): 1 over the block threshold, 0 warned, 0 not verified
Note the audit changes nothing — eject or update a flagged package yourself
```

| Flag | Effect |
|---|---|
| `--refresh` | Clears the verdict cache and re-queries the advisory service |
| `--no-scan` | Checks advisory data only; skips the artifact re-scan |
| `--fail-on block\|warn` | Exit code for CI (see below) |
| `--format json\|markdown\|sarif` | Renders the report in another format (see [Export formats](#export-formats)) |
| `--out FILE` | Writes the formatted report to FILE; requires `--format` |

A package whose extract directory has been swept reports `artifact not on disk —
nothing to re-scan`, which is distinct from a clean scan.

### `check` — Phase A only

The same path `audit --no-scan` takes, as its own verb: advisory data is
checked (honoring `--refresh` and `--fail-on`) and no extracted tree is walked.

### `scan` — Phase B only

Re-scans each installed package's extracted tree without any advisory lookup,
and reports per package `clean`, `N finding(s)`, `not on disk` (the extract
directory was swept) or `not scanned` (no install path recorded). `--json`
prints `{"command": "referee", "subcommand": "scan", "packages": [...]}` with
the same `scan` object `audit --json` uses.

### `--fail-on` exit codes

`--fail-on` turns the report into a CI gate without changing what the audit
does. It uses the install gate's own banding, so `block` fails on exactly the
packages an install would refuse.

| Flag | Exit 1 when |
|---|---|
| (none) | never — the audit always exits 0 when it runs |
| `--fail-on block` | any package is at or above `block_at` |
| `--fail-on warn` | any package is at or above `warn_at` (blocked packages included) |

The report (table, `--json` document or `--format` output) is printed first;
the failure is a `RefereeAuditFailed` error on stderr naming the packages.
`unverified` and `unknown` packages never trip `--fail-on`: they have no band.
Artifact-scan findings are reported but do not affect the exit code.

### `cache` — the verdict cache

| Flag | Effect |
|---|---|
| `--status` (default) | Row count per ecosystem and the newest `checked_at` (UTC) |
| `--clear` | Empties the cache — what `--refresh` does before an audit |
| `--prune <DAYS>` | Deletes verdicts computed more than DAYS days ago |

The flags are mutually exclusive. `--prune` is the manual answer to verdict
staleness; there is no automatic TTL.

### `config` — effective settings

Prints the `[referee]` settings actually in effect: `enabled` (which accounts
for `--no-referee`), `warn_at`, `block_at`, `fail_policy`, `osv_base_url`,
`virustotal_base_url` (`<default>` / `null` when unset) and
`virustotal_api_key: set|unset`. The key's value is never printed.
`--json` prints `{"command": "referee", "subcommand": "config", "config": {...}}`.

### `sbom` — CycloneDX inventory

Emits a CycloneDX 1.5 JSON document built from the roster alone — nothing is
fetched:

- one `components[]` entry per installed package: `type: application`,
  `bom-ref` (`name@version`), `name`, `version`, `description`, a `purl` for
  Cargo (`pkg:cargo/…`) and GitHub (`pkg:github/…`) packages, `hashes`
  (`SHA-256`, only when the roster holds a 64-hex digest),
  `externalReferences` (`distribution` = `download_url`, `vcs` = repository)
  and `properties` `baller:source` / `baller:user_installed`;
- one `dependencies[]` entry per component, with `dependsOn` built from the
  `package_dependencies` table. An edge to a package baller did not install (a
  system library, a virtual package) has no component and is left out.

SPDX output and license data are not produced: the roster stores no license.

---

## Export formats

`audit --format` renders the same audit three ways. With no `--out` the
rendered document replaces the table on stdout; with `--out FILE` it is written
to the file and stdout keeps the normal table (or `--json` document).

- **`json`** — the `baller referee --json` document described in
  [JSON output](#json-output).
- **`markdown`** — a `# Referee audit` document with the thresholds and fail
  policy, one table row per package (package, version, status, band, risk,
  advisories), a `## Details` section per package with advisories, unverified
  identities and scan findings, and the closing summary.
- **`sarif`** — a SARIF 2.1.0 log with one run whose `tool.driver` is
  `baller-referee`. `tool.driver.rules` holds one rule per advisory id
  (`helpUri` pointing at `osv.dev`), one per scan rule (`scan/<rule>`), and
  `referee/unverified`. `results` holds one result per matched advisory, per
  scan finding, and per unverified/unknown package; each carries `ruleId`,
  `ruleIndex`, a package `logicalLocation` (plus the file's
  `physicalLocation` for a scan finding) and `package`/`version`/`source`
  properties.

| Source | SARIF `level` |
|---|---|
| Advisory banded `block` on its own CVSS | `error` |
| Advisory banded `warn` (including unscored) | `warning` |
| Advisory banded `pass` | `note` |
| Scan finding `block` / `warn` | `error` / `warning` |
| Unverified or unknown package | `note` |

---

## Configuration

```ini
[referee]
enabled = true
warn_at = 2.5
block_at = 4.0
fail_policy = fail-open
osv_base_url = https://api.osv.dev
# virustotal_api_key = <your key>
# virustotal_base_url = https://www.virustotal.com/api/v3
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `true` | Master switch for both phases. `referee_enabled` is an accepted alias |
| `warn_at` | float 0–5 | `2.5` | Risk index at or above which a package is reported |
| `block_at` | float 0–5 | `4.0` | Risk index at or above which the plan is aborted |
| `fail_policy` | `fail-open` / `fail-closed` | `fail-open` | What an unreachable advisory service means. `open` and `closed` are accepted, and `_` is read as `-` |
| `osv_base_url` | string | `https://api.osv.dev` | Advisory API base URL, for self-hosting and tests |
| `virustotal_api_key` | string | unset | Enables the hash-only VirusTotal lookup |
| `virustotal_base_url` | string | `https://www.virustotal.com/api/v3` | VirusTotal API base URL, for tests and self-hosted proxies |

`0 <= warn_at < block_at <= 5` is validated when the config is parsed:

```
$ baller roster
[Error]: referee warn_at (4.5) must be below block_at (4)
```

`--no-referee` overrides `enabled = true` for one command. `baller referee`
itself refuses to run when Referee is disabled:

```
$ baller --no-referee referee
[Error]: referee is disabled — remove --no-referee, or set 'enabled = true' under [referee] in baller.conf
```

---

## Declaring an advisory identity

A package can state where its known-issue surface lives. In a `baller build`
manifest:

```toml
name = "ripgrep"
version = "14.1.1"

[advisory]
ecosystem = "crates.io"       # OSV ecosystem: crates.io, npm, NuGet, PyPI, GitHub, …
name = "ripgrep"              # optional; defaults to the package name
aliases = ["CVE-2026-1234"]   # advisory ids this package is tracked under
```

The same fields work in a JSON manifest and in registry-served metadata:

```json
{ "name": "ripgrep", "version": "14.1.1",
  "advisory": { "ecosystem": "crates.io", "aliases": ["CVE-2026-1234"] } }
```

- `ecosystem` + `name` become a `Declared` identity, queried in the same batch
  as any primary one. A section without `ecosystem` names nothing queryable and
  is ignored.
- Each entry in `aliases` is fetched by id and range-checked against the
  installed version, so an alias reports its issue on the versions it actually
  affects. Aliases are reported under the identity `declared:<package name>`.
- The declaration is stored on the roster (the `advisory` column of
  `installed_packages`), so `baller referee` re-checks the package under the
  same identity the install used.

A registry may instead ship OSV-shaped records with the metadata:

```json
{
  "name": "tool",
  "version": "2.0.0",
  "vulnerabilities": [
    {
      "id": "BALLER-2026-0001",
      "summary": "the registry knows about this one",
      "severity": [{ "type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H" }],
      "affected": [{ "ranges": [{ "type": "SEMVER", "events": [{ "introduced": "0" }] }] }]
    }
  ]
}
```

These are evaluated locally under the identity `BallerRegistry:<name>` and cost
no network request. Records that cannot be decoded are skipped individually and
logged at `--verbose`.

---

## Caching

Verdicts live in the existing SQLite database:

```sql
CREATE TABLE referee_cache (
  ecosystem   TEXT NOT NULL,
  name        TEXT NOT NULL,
  version     TEXT NOT NULL,
  verdict     TEXT NOT NULL,   -- clean | vulnerable
  risk        REAL,
  advisories  TEXT NOT NULL,   -- JSON array of matched advisories
  checked_at  TEXT NOT NULL,
  PRIMARY KEY (ecosystem, name, version)
);
```

- A row is only ever reused for the exact `(ecosystem, name, version)` it was
  computed from.
- Only `clean` and `vulnerable` are stored. `unknown` and `unverified` are not
  cached.
- A cache hit makes no network request; `--verbose` logs the hit and the date
  the verdict was computed.
- A row whose `verdict` this build does not recognise is read as `unknown` and
  re-queried.
- `baller referee --refresh` and `baller referee cache --clear` empty the
  table; `baller referee cache --prune <DAYS>` drops rows older than DAYS days
  and `baller referee cache` shows row counts per ecosystem.

Cache rows survive `baller sweep`, which clears downloads rather than database
state.

---

## JSON output

`--json` embeds the report in each command's single JSON document:

```json
{
  "command": "draft",
  "package": "badtool",
  "version": "2.0.0",
  "referee": {
    "enabled": true,
    "warn_at": 2.5,
    "block_at": 4.0,
    "packages": [
      {
        "name": "badtool",
        "version": "2.0.0",
        "source": "baller",
        "status": "vulnerable",
        "band": "block",
        "risk": 4.9,
        "advisories": [
          { "id": "GHSA-crit", "aliases": ["CVE-2026-0001"], "cvss": 9.8,
            "summary": "badtool is affected" }
        ],
        "identities": [
          { "ecosystem": "crates.io", "name": "badtool", "scope": "declared",
            "status": "vulnerable", "risk": 4.9, "advisories": [] }
        ]
      }
    ]
  }
}
```

`enabled` is `false` when Referee was skipped. `risk` and `cvss` are `null` when
nothing was scored, and are rounded to two and one decimal places respectively.
`advisories` is flattened across identities, worst first; `identities` keeps the
per-identity breakdown.

`baller referee --json` uses the same package shape, adds `fail_policy` at the
top level, and gives each package a `scan` object:

```json
{ "scan": { "scanned": true, "findings": [
    { "path": "bootstrap.sh", "rule": "suspicious-payload", "severity": "warn",
      "evidence": "curl -fsSL https://x.test | sh" } ] } }
```

`{"scanned": false}` means `--no-scan` or a source with no artifact;
`{"scanned": false, "reason": "install path is gone"}` means the extract
directory is no longer on disk.

---

## Errors

| Variant | Raised when | State left behind |
|---|---|---|
| `RefereeBlocked` | A package crosses `block_at` in Phase A | Nothing written: no download, no symlink, no roster row |
| `RefereeScanBlocked` | Phase B finds a block-severity issue | Extract directory and cached archive both purged |
| `RefereeUnavailable` | The advisory service is unreachable under `fail-closed` | Nothing written |
| `RefereeAuditFailed` | `baller referee audit/check --fail-on` found a package at or above the band | Nothing written — the report is printed first; only the exit code changes |

`RefereeBlocked` lists every package over the threshold with its advisories and
reason, and names the escape hatches. All four exit non-zero, like any other
failed command. See [error-handling.md](error-handling.md#referee-errors).

---

## Code map

Referee lives in `src/security/`:

| File | Contents |
|---|---|
| `mod.rs` | `Referee` (the service on `AppContext`), `Referee::gate`, `Referee::audit`, `Referee::screen_artifact`, `GateOutcome`, `FailPolicy` |
| `export.rs` | `ScanOutcome`, `render_markdown`, `render_sarif` — the audit's Markdown and SARIF 2.1.0 writers |
| `identity.rs` | `AdvisoryIdentity`, `IdentityScope`, `advisory_identities(&Package)` |
| `osv.rs` | `OsvClient` (`query_batch`, `vuln`), and the wire types `Vulnerability`, `Severity`, `Affected`, `Range`, `Event` |
| `ranges.rs` | `affects`, `entry_affects`, `entry_is_about` — affected-version interval matching |
| `scoring.rs` | `cvss_score`, `best_cvss`, `qualitative_cvss`, `risk_index`, `classify`, `Band`, `RefereeThresholds` |
| `verdict.rs` | `Verdict`, `MatchedAdvisory`, `AdvisoryVerdict`, `PackageReport` |
| `scan.rs` | `ArtifactScanner`, `ScanRule`, `ScanSeverity`, `ScanFinding`, `has_blocking`, `executable_candidates`, `shannon_entropy` |
| `virustotal.rs` | `VirusTotalClient::screen` — the hash-only lookup |

Entry points elsewhere:

| Location | What it does |
|---|---|
| `src/context.rs` | Builds `Referee` from `config.referee` and `GlobalFlags::no_referee` |
| `src/config/config.rs` | `RefereeConfig`, the `[referee]` keys, threshold validation |
| `src/commands/draft.rs` | Phase A before the install loop; `screen_artifact()` wraps Phase B and purges on a block |
| `src/commands/update.rs` | Phase A on the upgrade plan; Phase B before the old extract directory is pruned |
| `src/commands/substitute.rs` | Phase A before the old package is ejected; Phase B before linking |
| `src/commands/referee/` | The `baller referee` group: `mod.rs` (dispatcher, roster resolution, re-scan, table, `fail_on_error`), `audit.rs`, `check.rs`, `scan.rs`, `cache.rs`, `config.rs`, `sbom.rs` (CycloneDX writer), `export.rs` (stdout / `--out` delivery) |
| `src/cli/parse.rs` | `RefereeArgs` / `RefereeSub` — the clap group and the bare-form back-compat |
| `src/core/db.rs` | `referee_cache_get` / `_put` / `_clear` / `_count` / `_stats` / `_prune_older_than`, the `advisory` roster column, `InstalledPackage::to_package` |
| `src/core/package.rs` | `AdvisoryDeclaration`, `Package::advisory_identities`, `Package::declared_aliases` |
| `src/error/error.rs` | `RefereeBlocked`, `RefereeScanBlocked`, `RefereeUnavailable`, `RefereeAuditFailed`, `BlockedPackage` |
| `src/http/mod.rs` | `HttpClient::post_json`, `HttpClient::get_json_optional_with_headers` |

### Tests

Unit tests sit beside each module. `src/security/integration.rs` runs the whole
service against a mock advisory server built on `TcpListener`: banding,
blocking, fail-open and fail-closed, cache hits and misses, batch chunking and
result alignment, hydration, withdrawn and out-of-range advisories, declared
aliases, registry-native records, both Phase B outcomes, and the `--fail-on`
exit-code banding.

`src/benches/workflow.rs` carries `referee_phase_a_cached` (a 25-package cached
gate) and `referee_phase_b_scan` (a small extracted tree).

---

## Scope and limits

Referee reports known vulnerabilities and a small set of artifact signals. It is
not an antivirus, not a guarantee against a novel supply-chain attack, and not
a license scanner. `baller referee sbom` inventories what is installed, but
carries no license data and produces CycloneDX only (no SPDX).

Known gaps:

- **`GitHub` ecosystem coverage.** The repo-scoped `(GitHub, owner/repo)`
  identity behaves identically on Linux and Windows, but public advisory
  coverage under a bare repo identity is thinner than under the language
  ecosystems. A `clean` verdict there means "no record". A package that also
  publishes to a language ecosystem should declare it with `[advisory]`.
- **Distro identities are best-effort.** Debian and Fedora versions are
  normalised heuristically, and distro *source* package names differ from binary
  ones — hence `Fallback` scope. `pacman` maps to no OSV ecosystem and reports
  `unknown`.
- **Chocolatey ids with no NuGet record and no GitHub `project_url`** map to
  nothing and report `unknown`.
- **CVSS v4.0 vectors** are not computed from the vector; such advisories fall
  back to a qualitative rating, and warn when there is none.
- **The Baller registry serves no advisory data yet**
  ([#10](https://github.com/HMythical/baller/issues/10)), so registry-sourced
  packages are `unknown` unless they carry `[advisory]` or `vulnerabilities`.
- **`baller build` is not gated.** Referee runs in `draft`, `update` and
  `substitute`; a package assembled from a local manifest is not checked.
