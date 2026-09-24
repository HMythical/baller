//! Referee — the neutral arbiter between the user and a package.
//!
//! Referee runs in two phases, and they answer different questions:
//!
//! * **Phase A, the advisory gate** ([`Referee::gate`]) asks public advisory
//!   data whether anything is *known* about the packages a command intends to
//!   install. It runs once, up front, on the whole resolved plan — before the
//!   install loop writes anything — so a block is atomic: nothing is
//!   symlinked, recorded or handed to `sudo`.
//! * **Phase B, the artifact scan** ([`Referee::screen_artifact`]) asks the
//!   downloaded archive itself, because a freshly backdoored release has no
//!   advisory yet. It runs after extraction and before linking, so a flagged
//!   artifact is purged instead of installed.
//!
//! Two invariants hold throughout:
//!
//! * **Absence of data is never safety.** A package nobody has a record for is
//!   `Unknown`, an outage is `Unverified`, and both are reported rather than
//!   quietly passed.
//! * **An unreachable advisory service does not brick an install.** The default
//!   `fail_policy` is fail-open; `fail-closed` is available for users who would
//!   rather stop than proceed unverified.

#[cfg(test)]
mod integration;

pub mod export;
pub mod identity;
pub mod osv;
pub mod ranges;
pub mod scan;
pub mod scoring;
pub mod verdict;
pub mod virustotal;

use std::collections::HashMap;
use std::path::Path;

use colored::Colorize;
use serde_json::{json, Value};

use crate::core::db::DbManager;
use crate::core::package::{Package, PackageSource};
use crate::error::error::{BallError, BlockedPackage};
use crate::http::HttpClient;
use crate::security::identity::{advisory_identities, AdvisoryIdentity, IdentityScope};
use crate::security::osv::{OsvClient, OsvQuery, Vulnerability};
use crate::security::scan::{ArtifactScanner, ScanFinding};
use crate::security::scoring::{best_cvss, qualitative_cvss, risk_index, Band, RefereeThresholds};
use crate::security::verdict::{AdvisoryVerdict, MatchedAdvisory, PackageReport, Verdict};

/// The ecosystem name a registry's own advisory feed is reported under.
const ECOSYSTEM_REGISTRY: &str = "BallerRegistry";

/// The identity a package's own declared advisory ids are reported under.
const ECOSYSTEM_DECLARED: &str = "declared";

/// What to do when the advisory service cannot be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailPolicy {
    /// Report the packages as `Unverified` and let the install proceed
    FailOpen,
    /// Refuse to install what could not be checked
    FailClosed,
}

impl FailPolicy {
    pub fn label(self) -> &'static str {
        match self {
            FailPolicy::FailOpen => "fail-open",
            FailPolicy::FailClosed => "fail-closed",
        }
    }

    /// Parse the `fail_policy` config value.
    pub fn from_config_value(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().replace('_', "-").as_str() {
            "fail-open" | "open" => Some(FailPolicy::FailOpen),
            "fail-closed" | "closed" => Some(FailPolicy::FailClosed),
            _ => None,
        }
    }
}

/// The security service commands call into.
pub struct Referee {
    enabled: bool,
    osv: OsvClient,
    thresholds: RefereeThresholds,
    fail_policy: FailPolicy,
    scanner: ArtifactScanner,
    virustotal: Option<virustotal::VirusTotalClient>,
}

impl Referee {
    pub fn new(
        http: HttpClient,
        enabled: bool,
        thresholds: RefereeThresholds,
        fail_policy: FailPolicy,
        osv_base_url: String,
        virustotal_api_key: Option<String>,
        virustotal_base_url: Option<String>,
    ) -> Self {
        let virustotal = virustotal_api_key
            .filter(|key| !key.trim().is_empty())
            .map(|key| {
                let client = virustotal::VirusTotalClient::new(http.clone(), key);
                match virustotal_base_url.filter(|url| !url.trim().is_empty()) {
                    Some(url) => client.with_base_url(url),
                    None => client,
                }
            });

        Self {
            enabled,
            osv: OsvClient::new(http, osv_base_url),
            thresholds,
            fail_policy,
            scanner: ArtifactScanner::new(),
            virustotal,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn thresholds(&self) -> &RefereeThresholds {
        &self.thresholds
    }

    pub fn fail_policy(&self) -> FailPolicy {
        self.fail_policy
    }

    /// Phase A: check a whole install plan before any of it is installed.
    ///
    /// The result is a report, not a decision — the caller decides what a block
    /// means, because `draft --dry-run` wants to *show* a block where a real
    /// install wants to *be* one.
    pub fn gate(&self, db: &DbManager, packages: &[Package]) -> Result<GateOutcome, BallError> {
        if !self.enabled {
            tracing::debug!("referee: disabled, skipping the advisory gate");
            return Ok(GateOutcome::skipped(self.thresholds));
        }

        if packages.is_empty() {
            return Ok(GateOutcome::new(Vec::new(), self.thresholds));
        }

        self.check(db, packages, false)
    }

    /// Re-check already-installed packages for the `referee` audit command.
    ///
    /// `refresh` bypasses the cache: a `Clean` verdict recorded last month was
    /// computed against the advisory data of last month, and an audit is
    /// exactly when a user wants that re-asked.
    pub fn audit(
        &self,
        db: &DbManager,
        packages: &[Package],
        refresh: bool,
    ) -> Result<GateOutcome, BallError> {
        self.check(db, packages, refresh)
    }

    fn check(
        &self,
        db: &DbManager,
        packages: &[Package],
        refresh: bool,
    ) -> Result<GateOutcome, BallError> {
        // Every identity every package can be looked up under, kept alongside
        // the package it came from.
        let mut plan: Vec<(usize, Vec<AdvisoryIdentity>)> = Vec::with_capacity(packages.len());
        for (index, pkg) in packages.iter().enumerate() {
            plan.push((index, identities_for(pkg)));
        }

        // Resolve as much as the cache can answer, and note what it cannot.
        let mut resolved: HashMap<(usize, usize), AdvisoryVerdict> = HashMap::new();
        let mut queries: Vec<OsvQuery> = Vec::new();
        let mut pending: Vec<(usize, usize)> = Vec::new();

        for (index, identities) in &plan {
            let pkg = &packages[*index];

            for (slot, identity) in identities.iter().enumerate() {
                // Registry-served advisories need no network call at all: the
                // registry already stated what it knows about this version.
                if identity.ecosystem == ECOSYSTEM_REGISTRY {
                    resolved.insert(
                        (*index, slot),
                        self.evaluate_registry_advisories(pkg, identity),
                    );
                    continue;
                }

                if !refresh {
                    match db.referee_cache_get(&identity.ecosystem, &identity.name, &pkg.version) {
                        Ok(Some(cached)) => {
                            tracing::debug!(
                                "referee: cache hit for {} v{} (checked {})",
                                identity.label(),
                                pkg.version,
                                cached.checked_at
                            );
                            resolved.insert((*index, slot), cached.into_verdict(identity.clone()));
                            continue;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::debug!("referee: cache lookup failed: {}", e);
                        }
                    }
                }

                queries.push(OsvQuery {
                    ecosystem: identity.ecosystem.clone(),
                    name: identity.name.clone(),
                    version: pkg.version.clone(),
                });
                pending.push((*index, slot));
            }
        }

        if !queries.is_empty() {
            tracing::debug!(
                "referee: querying {} for {} identit(ies)",
                self.osv.base_url(),
                queries.len()
            );

            match self.osv.query_batch(&queries) {
                Ok(results) => {
                    let hydrated = self.hydrate(&results);
                    for (position, (index, slot)) in pending.iter().enumerate() {
                        let pkg = &packages[*index];
                        let identity = &plan[*index].1[*slot];
                        let vulns = results.get(position).map(Vec::as_slice).unwrap_or(&[]);
                        let advisory_verdict =
                            self.evaluate(identity, &pkg.version, vulns, &hydrated);

                        self.cache_verdict(db, identity, &pkg.version, &advisory_verdict);
                        resolved.insert((*index, *slot), advisory_verdict);
                    }
                }
                Err(e) => {
                    if self.fail_policy == FailPolicy::FailClosed {
                        return Err(BallError::RefereeUnavailable {
                            message: format!(
                                "{} (fail_policy = fail-closed, so nothing was installed)",
                                e
                            ),
                        });
                    }

                    tracing::debug!("referee: advisory lookup failed, failing open: {}", e);
                    for (index, slot) in &pending {
                        let identity = plan[*index].1[*slot].clone();
                        resolved.insert(
                            (*index, *slot),
                            AdvisoryVerdict {
                                identity,
                                status: Verdict::Unverified,
                                matched: Vec::new(),
                            },
                        );
                    }
                }
            }
        }

        let mut reports = Vec::with_capacity(packages.len());
        for (index, identities) in &plan {
            let pkg = &packages[*index];
            let source = crate::commands::draft::source_label(&pkg.source);

            let mut verdicts: Vec<AdvisoryVerdict> = (0..identities.len())
                .filter_map(|slot| resolved.remove(&(*index, slot)))
                .collect();

            if let Some(declared) = self.check_declared_aliases(pkg) {
                verdicts.push(declared);
            }

            if verdicts.is_empty() {
                tracing::debug!(
                    "referee: {} v{} maps to no advisory identity",
                    pkg.name,
                    pkg.version
                );
                reports.push(PackageReport::unknown(&pkg.name, &pkg.version, &source));
                continue;
            }

            reports.push(PackageReport {
                name: pkg.name.clone(),
                version: pkg.version.clone(),
                source,
                verdicts,
            });
        }

        Ok(GateOutcome::new(reports, self.thresholds))
    }

    /// Look up the advisory ids a package says it is tracked under.
    ///
    /// This is the other half of the manifest `[advisory]` section: a package
    /// whose distribution shape maps to no OSV ecosystem can still name the
    /// records that describe it, and those are fetched by id. The version is
    /// still range-checked against each record, so declaring an alias reports
    /// the issue on the versions it actually affects rather than forever.
    fn check_declared_aliases(&self, pkg: &Package) -> Option<AdvisoryVerdict> {
        let aliases: Vec<&String> = pkg
            .declared_aliases()
            .iter()
            .filter(|alias| !alias.trim().is_empty())
            .collect();

        if aliases.is_empty() {
            return None;
        }

        let identity = AdvisoryIdentity::new(
            ECOSYSTEM_DECLARED,
            pkg.name.clone(),
            IdentityScope::Declared,
        );

        let mut matched = Vec::new();
        let mut failed = false;

        for alias in aliases {
            match self.osv.vuln(alias.trim()) {
                Ok(Some(record)) => {
                    if record.is_withdrawn() {
                        continue;
                    }
                    // The author asserted that this record describes their
                    // package, so its `affected` entries are read for the
                    // versions they name, not for the package they name — a
                    // record filed against another ecosystem's spelling of the
                    // same project would otherwise be discarded here.
                    let covers = record.affected.is_empty()
                        || record
                            .affected
                            .iter()
                            .any(|entry| ranges::entry_covers_version(entry, &pkg.version))
                        || record
                            .affected
                            .iter()
                            .all(ranges::entry_has_no_version_bound);

                    if !covers {
                        tracing::debug!(
                            "referee: declared alias {} does not cover {} v{}",
                            record.id,
                            pkg.name,
                            pkg.version
                        );
                        continue;
                    }

                    matched.push(MatchedAdvisory {
                        id: record.id.clone(),
                        aliases: record.aliases.clone(),
                        cvss: score_of(&record),
                        summary: record.short_summary(),
                    });
                }
                Ok(None) => {
                    tracing::debug!("referee: declared alias {} has no record", alias);
                }
                Err(e) => {
                    tracing::debug!("referee: could not fetch declared alias {}: {}", alias, e);
                    failed = true;
                }
            }
        }

        let status = if !matched.is_empty() {
            Verdict::Vulnerable {
                risk: highest_risk(&matched),
            }
        } else if failed {
            Verdict::Unverified
        } else {
            Verdict::Clean
        };

        Some(AdvisoryVerdict {
            identity,
            status,
            matched,
        })
    }

    /// Fetch the full record for every advisory that a batch answer named but
    /// did not describe.
    ///
    /// `querybatch` answers with ids only, so this is where severity, summary
    /// and affected ranges actually arrive. It is keyed by id so an advisory
    /// affecting five packages in one plan is fetched once, and a failure on
    /// one id is recorded and stepped over rather than failing the gate.
    fn hydrate(&self, results: &[Vec<Vulnerability>]) -> HashMap<String, Vulnerability> {
        let mut out: HashMap<String, Vulnerability> = HashMap::new();

        for vulns in results {
            for vuln in vulns {
                if vuln.id.is_empty() || out.contains_key(&vuln.id) {
                    continue;
                }

                if vuln.is_detailed() {
                    out.insert(vuln.id.clone(), vuln.clone());
                    continue;
                }

                match self.osv.vuln(&vuln.id) {
                    Ok(Some(full)) => {
                        out.insert(vuln.id.clone(), full);
                    }
                    Ok(None) => {
                        tracing::debug!("referee: {} has no detail record", vuln.id);
                    }
                    Err(e) => {
                        // The batch already said this version is affected. Not
                        // being able to read the detail costs the score and the
                        // summary, not the finding itself.
                        tracing::debug!("referee: could not fetch {}: {}", vuln.id, e);
                    }
                }
            }
        }

        out
    }

    /// Turn one identity's raw advisory list into a verdict.
    fn evaluate(
        &self,
        identity: &AdvisoryIdentity,
        version: &str,
        vulns: &[Vulnerability],
        hydrated: &HashMap<String, Vulnerability>,
    ) -> AdvisoryVerdict {
        let mut matched = Vec::new();

        for vuln in vulns {
            let record = hydrated.get(&vuln.id).unwrap_or(vuln);

            if record.is_withdrawn() {
                tracing::debug!("referee: {} is withdrawn, ignoring", record.id);
                continue;
            }

            if !applies_to(record, identity, version) {
                tracing::debug!(
                    "referee: {} does not cover {} v{} on a local range check",
                    record.id,
                    identity.label(),
                    version
                );
                continue;
            }

            matched.push(MatchedAdvisory {
                id: record.id.clone(),
                aliases: record.aliases.clone(),
                cvss: score_of(record),
                summary: record.short_summary(),
            });
        }

        let status = if matched.is_empty() {
            Verdict::Clean
        } else {
            Verdict::Vulnerable {
                risk: highest_risk(&matched),
            }
        };

        AdvisoryVerdict {
            identity: identity.clone(),
            status,
            matched,
        }
    }

    /// Evaluate the advisories a registry shipped with the package metadata.
    ///
    /// Malformed entries are dropped one by one and reported at `--verbose`:
    /// one unreadable record in a registry's feed must not discard the rest.
    fn evaluate_registry_advisories(
        &self,
        pkg: &Package,
        identity: &AdvisoryIdentity,
    ) -> AdvisoryVerdict {
        let mut matched = Vec::new();

        for raw in &pkg.vulnerabilities {
            let record: Vulnerability = match serde_json::from_value(raw.clone()) {
                Ok(record) => record,
                Err(e) => {
                    tracing::debug!(
                        "referee: {} shipped an unreadable advisory record: {}",
                        pkg.name,
                        e
                    );
                    continue;
                }
            };

            if record.id.is_empty() || record.is_withdrawn() {
                continue;
            }

            // A registry-native record is about this package by construction,
            // so an entry with no usable range is still a statement about this
            // version rather than a mismatch to discard.
            let covered = record.affected.is_empty()
                || ranges::affects(
                    &record.affected,
                    &identity.ecosystem,
                    &identity.name,
                    &pkg.version,
                )
                || ranges::affects(
                    &record.affected,
                    &identity.ecosystem,
                    &pkg.name,
                    &pkg.version,
                );

            if !covered {
                continue;
            }

            matched.push(MatchedAdvisory {
                id: record.id.clone(),
                aliases: record.aliases.clone(),
                cvss: score_of(&record),
                summary: record.short_summary(),
            });
        }

        let status = if matched.is_empty() {
            Verdict::Clean
        } else {
            Verdict::Vulnerable {
                risk: highest_risk(&matched),
            }
        };

        AdvisoryVerdict {
            identity: identity.clone(),
            status,
            matched,
        }
    }

    /// Record a verdict for next time.
    ///
    /// Only answers are cached. `Unverified` is an outage and `Unknown` is a
    /// missing mapping; storing either would turn a transient failure into a
    /// durable one.
    fn cache_verdict(
        &self,
        db: &DbManager,
        identity: &AdvisoryIdentity,
        version: &str,
        verdict: &AdvisoryVerdict,
    ) {
        if verdict.status.is_unchecked() {
            return;
        }

        let advisories =
            serde_json::to_string(&verdict.matched).unwrap_or_else(|_| "[]".to_string());
        if let Err(e) = db.referee_cache_put(
            &identity.ecosystem,
            &identity.name,
            version,
            verdict.status.label(),
            verdict.status.risk(),
            &advisories,
        ) {
            tracing::debug!("referee: could not cache a verdict: {}", e);
        }
    }

    /// Phase B: look at what was actually downloaded.
    ///
    /// `Ok` carries the warn-level findings for the caller to print; `Err` is
    /// [`BallError::RefereeScanBlocked`], and the caller purges the download.
    pub fn screen_artifact(
        &self,
        pkg: &Package,
        extract_dir: &Path,
    ) -> Result<Vec<ScanFinding>, BallError> {
        if !self.enabled {
            return Ok(Vec::new());
        }

        // System and cargo packages are installed by native tooling; no
        // artifact passes through baller, so there is nothing to scan.
        if matches!(
            pkg.source,
            PackageSource::System { .. } | PackageSource::Cargo { .. }
        ) {
            return Ok(Vec::new());
        }

        tracing::debug!(
            "referee: scanning {} for {} v{}",
            extract_dir.display(),
            pkg.name,
            pkg.version
        );

        let mut findings = self.scanner.scan_tree(extract_dir);

        if let Some(virustotal) = self.virustotal.as_ref() {
            let candidates = scan::executable_candidates(extract_dir);
            tracing::debug!(
                "referee: submitting {} hash(es) to virustotal",
                candidates.len()
            );
            findings.extend(virustotal.screen(&candidates, extract_dir));
        }

        if scan::has_blocking(&findings) {
            return Err(BallError::RefereeScanBlocked {
                package: pkg.name.clone(),
                version: pkg.version.clone(),
                findings,
            });
        }

        Ok(findings)
    }

    /// Print the warn-level findings from a scan that did not block.
    pub fn report_scan_findings(&self, pkg: &Package, findings: &[ScanFinding]) {
        for finding in findings {
            tracing::info!(
                "{} {} v{}: {} in {} — {}",
                "Referee".yellow().bold(),
                pkg.name.cyan(),
                pkg.version.yellow(),
                finding.rule.label(),
                finding.path.display().to_string().cyan(),
                finding.evidence
            );
        }
    }
}

/// The identities a package is checked under, including its registry's own feed.
fn identities_for(pkg: &Package) -> Vec<AdvisoryIdentity> {
    let mut identities = advisory_identities(pkg);

    if !pkg.vulnerabilities.is_empty() {
        identities.insert(
            0,
            AdvisoryIdentity::new(ECOSYSTEM_REGISTRY, pkg.name.clone(), IdentityScope::Primary),
        );
    }

    identities
}

/// Whether a record really covers this identity's version.
///
/// The batch endpoint already filters by version, so this is a second opinion
/// rather than the only one — and it is applied only when the record actually
/// names the identity being checked. A record that describes the package under
/// a naming scheme baller does not model (a purl, an ecosystem variant) is kept
/// on the service's word, because discarding it would turn an unfamiliar
/// spelling into a silent all-clear.
fn applies_to(record: &Vulnerability, identity: &AdvisoryIdentity, version: &str) -> bool {
    if record.affected.is_empty() {
        return true;
    }

    let names_identity = record
        .affected
        .iter()
        .any(|entry| ranges::entry_is_about(entry, &identity.ecosystem, &identity.name));

    if !names_identity {
        return true;
    }

    ranges::affects(
        &record.affected,
        &identity.ecosystem,
        &identity.name,
        version,
    )
}

/// The CVSS score for a record, from the most specific source available.
fn score_of(record: &Vulnerability) -> Option<f32> {
    best_cvss(&record.severity)
        .or_else(|| {
            record
                .affected
                .iter()
                .filter_map(|entry| best_cvss(&entry.severity))
                .fold(None, |best: Option<f32>, score| {
                    Some(best.map_or(score, |best| best.max(score)))
                })
        })
        .or_else(|| qualitative_cvss(record.database_specific.as_ref()))
}

/// The highest risk index across a set of matched advisories.
fn highest_risk(matched: &[MatchedAdvisory]) -> Option<f32> {
    matched
        .iter()
        .filter_map(|advisory| advisory.cvss)
        .map(risk_index)
        .fold(None, |best: Option<f32>, risk| {
            Some(best.map_or(risk, |best| best.max(risk)))
        })
}

/// What Phase A concluded about a whole plan.
pub struct GateOutcome {
    pub reports: Vec<PackageReport>,
    pub thresholds: RefereeThresholds,
    /// True when Referee was switched off and nothing was checked
    pub skipped: bool,
}

impl GateOutcome {
    pub fn new(reports: Vec<PackageReport>, thresholds: RefereeThresholds) -> Self {
        Self {
            reports,
            thresholds,
            skipped: false,
        }
    }

    pub fn skipped(thresholds: RefereeThresholds) -> Self {
        Self {
            reports: Vec::new(),
            thresholds,
            skipped: true,
        }
    }

    pub fn in_band(&self, band: Band) -> Vec<&PackageReport> {
        self.reports
            .iter()
            .filter(|report| report.band(&self.thresholds) == band)
            .collect()
    }

    pub fn blocked(&self) -> Vec<&PackageReport> {
        self.in_band(Band::Block)
    }

    pub fn warned(&self) -> Vec<&PackageReport> {
        self.in_band(Band::Warn)
    }

    /// Packages nothing could be confirmed about, for the closing summary.
    pub fn unchecked(&self) -> Vec<&PackageReport> {
        self.reports
            .iter()
            .filter(|report| report.status().is_unchecked())
            .collect()
    }

    /// The error a real install returns when the gate blocked.
    pub fn block_error(&self) -> Option<BallError> {
        let blocked = self.blocked();
        if blocked.is_empty() {
            return None;
        }

        Some(BallError::RefereeBlocked {
            packages: blocked
                .iter()
                .map(|report| BlockedPackage {
                    package: report.name.clone(),
                    version: report.version.clone(),
                    advisories: report
                        .advisories()
                        .into_iter()
                        .cloned()
                        .collect::<Vec<MatchedAdvisory>>(),
                    reason: report.block_reason(&self.thresholds),
                })
                .collect(),
        })
    }

    /// Print what the user needs to see: every warned package, and a one-line
    /// summary of what could not be verified.
    ///
    /// Blocked packages are not printed here — they become the error, and
    /// printing them twice would only bury the reason.
    pub fn report(&self, quiet: bool) {
        if self.skipped || quiet {
            return;
        }

        for report in self.warned() {
            tracing::info!(
                "{} {} v{}: {}",
                "Referee".yellow().bold(),
                report.name.cyan(),
                report.version.yellow(),
                risk_phrase(report.risk())
            );
            for advisory in report.advisories() {
                tracing::info!("    {} {}", "•".yellow(), advisory.describe());
            }
        }

        let unchecked = self.unchecked();
        if !unchecked.is_empty() {
            let names: Vec<String> = unchecked
                .iter()
                .map(|report| format!("{} ({})", report.name, report.status().label()))
                .collect();
            tracing::info!(
                "{} {} package(s) were not verified: {}",
                "Referee".yellow().bold(),
                unchecked.len(),
                names.join(", ")
            );
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "enabled": !self.skipped,
            "warn_at": self.thresholds.warn_at,
            "block_at": self.thresholds.block_at,
            "packages": self.reports.iter().map(PackageReport::to_json).collect::<Vec<_>>(),
        })
    }
}

/// How a warned package's risk is phrased in a report line.
fn risk_phrase(risk: Option<f32>) -> String {
    match risk {
        Some(risk) => format!("risk index {:.2} of 5.00", risk),
        None => "an advisory matched, with no severity published".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::package::AdvisoryDeclaration;
    use crate::security::osv::{Affected, AffectedPackage, Event, Range, Severity};

    fn vuln(id: &str, cvss: Option<&str>) -> Vulnerability {
        Vulnerability {
            id: id.to_string(),
            severity: cvss
                .map(|score| {
                    vec![Severity {
                        kind: "CVSS_V3".to_string(),
                        score: score.to_string(),
                    }]
                })
                .unwrap_or_default(),
            ..Default::default()
        }
    }

    fn crates_identity(name: &str) -> AdvisoryIdentity {
        AdvisoryIdentity::new("crates.io", name, IdentityScope::Primary)
    }

    #[test]
    fn test_fail_policy_parsing() {
        assert_eq!(
            FailPolicy::from_config_value("fail-open"),
            Some(FailPolicy::FailOpen)
        );
        assert_eq!(
            FailPolicy::from_config_value("FAIL_CLOSED"),
            Some(FailPolicy::FailClosed)
        );
        assert_eq!(
            FailPolicy::from_config_value(" closed "),
            Some(FailPolicy::FailClosed)
        );
        assert_eq!(FailPolicy::from_config_value("maybe"), None);
    }

    #[test]
    fn test_fail_policy_labels() {
        assert_eq!(FailPolicy::FailOpen.label(), "fail-open");
        assert_eq!(FailPolicy::FailClosed.label(), "fail-closed");
    }

    #[test]
    fn test_score_of_prefers_the_record_severity() {
        let record = vuln("X", Some("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"));
        assert!((score_of(&record).unwrap() - 9.8).abs() < 0.05);
    }

    #[test]
    fn test_score_of_falls_back_to_the_affected_entry() {
        let mut record = vuln("X", None);
        record.affected.push(Affected {
            severity: vec![Severity {
                kind: "CVSS_V3".to_string(),
                score: "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:N".to_string(),
            }],
            ..Default::default()
        });
        assert!((score_of(&record).unwrap() - 7.5).abs() < 0.05);
    }

    #[test]
    fn test_score_of_falls_back_to_a_qualitative_rating() {
        let mut record = vuln("X", None);
        record.database_specific = Some(json!({ "severity": "CRITICAL" }));
        assert_eq!(score_of(&record), Some(9.0));
    }

    #[test]
    fn test_score_of_is_none_when_nothing_is_published() {
        assert_eq!(score_of(&vuln("X", None)), None);
    }

    #[test]
    fn test_highest_risk_takes_the_maximum() {
        let matched = vec![
            MatchedAdvisory {
                id: "a".to_string(),
                aliases: Vec::new(),
                cvss: Some(4.0),
                summary: None,
            },
            MatchedAdvisory {
                id: "b".to_string(),
                aliases: Vec::new(),
                cvss: Some(9.0),
                summary: None,
            },
            MatchedAdvisory {
                id: "c".to_string(),
                aliases: Vec::new(),
                cvss: None,
                summary: None,
            },
        ];
        assert_eq!(highest_risk(&matched), Some(4.5));
    }

    #[test]
    fn test_highest_risk_is_none_when_nothing_is_scored() {
        let matched = vec![MatchedAdvisory {
            id: "a".to_string(),
            aliases: Vec::new(),
            cvss: None,
            summary: None,
        }];
        assert_eq!(highest_risk(&matched), None);
    }

    #[test]
    fn test_applies_to_accepts_a_record_with_no_affected_block() {
        let record = vuln("X", None);
        assert!(applies_to(&record, &crates_identity("serde"), "1.0.0"));
    }

    #[test]
    fn test_applies_to_rejects_a_version_outside_the_range() {
        let mut record = vuln("X", None);
        record.affected.push(Affected {
            package: Some(AffectedPackage {
                ecosystem: "crates.io".to_string(),
                name: "serde".to_string(),
                purl: None,
            }),
            ranges: vec![Range {
                kind: "SEMVER".to_string(),
                events: vec![
                    Event {
                        introduced: Some("1.0.0".to_string()),
                        ..Default::default()
                    },
                    Event {
                        fixed: Some("1.0.5".to_string()),
                        ..Default::default()
                    },
                ],
            }],
            ..Default::default()
        });

        assert!(applies_to(&record, &crates_identity("serde"), "1.0.1"));
        assert!(!applies_to(&record, &crates_identity("serde"), "1.0.9"));
    }

    #[test]
    fn test_applies_to_trusts_the_service_when_the_record_names_no_known_identity() {
        let mut record = vuln("X", None);
        record.affected.push(Affected {
            package: Some(AffectedPackage {
                ecosystem: "SomeOtherEcosystem".to_string(),
                name: "totally-different".to_string(),
                purl: None,
            }),
            ranges: vec![Range {
                kind: "SEMVER".to_string(),
                events: vec![Event {
                    introduced: Some("99.0.0".to_string()),
                    ..Default::default()
                }],
            }],
            ..Default::default()
        });

        // Nothing here describes crates.io:serde, so the batch answer stands.
        assert!(applies_to(&record, &crates_identity("serde"), "1.0.0"));
    }

    #[test]
    fn test_identities_for_puts_the_registry_feed_first() {
        let mut pkg = Package::new("tool", "1.0.0");
        pkg.source = PackageSource::BallerRegistry {
            url: "https://registry.baller.dev/api".to_string(),
        };
        pkg.vulnerabilities = vec![json!({ "id": "BALLER-1" })];
        pkg.advisory = Some(AdvisoryDeclaration {
            ecosystem: Some("crates.io".to_string()),
            name: Some("tool".to_string()),
            aliases: Vec::new(),
        });

        let identities = identities_for(&pkg);
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].ecosystem, ECOSYSTEM_REGISTRY);
        assert_eq!(identities[1].ecosystem, "crates.io");
    }

    #[test]
    fn test_identities_for_leaves_a_plain_package_alone() {
        let pkg = Package::new("tool", "1.0.0");
        assert!(identities_for(&pkg).is_empty());
    }

    #[test]
    fn test_risk_phrase_says_unscored_rather_than_zero() {
        assert!(risk_phrase(None).contains("no severity published"));
        assert!(risk_phrase(Some(3.5)).contains("3.50"));
    }

    #[test]
    fn test_gate_outcome_bands_and_block_error() {
        let thresholds = RefereeThresholds::default();
        let reports = vec![
            PackageReport {
                name: "safe".to_string(),
                version: "1.0.0".to_string(),
                source: "cargo:safe".to_string(),
                verdicts: vec![AdvisoryVerdict {
                    identity: crates_identity("safe"),
                    status: Verdict::Clean,
                    matched: Vec::new(),
                }],
            },
            PackageReport {
                name: "warned".to_string(),
                version: "1.0.0".to_string(),
                source: "cargo:warned".to_string(),
                verdicts: vec![AdvisoryVerdict {
                    identity: crates_identity("warned"),
                    status: Verdict::Vulnerable { risk: Some(3.0) },
                    matched: vec![MatchedAdvisory {
                        id: "GHSA-warn".to_string(),
                        aliases: Vec::new(),
                        cvss: Some(6.0),
                        summary: None,
                    }],
                }],
            },
            PackageReport {
                name: "blocked".to_string(),
                version: "2.0.0".to_string(),
                source: "cargo:blocked".to_string(),
                verdicts: vec![AdvisoryVerdict {
                    identity: crates_identity("blocked"),
                    status: Verdict::Vulnerable { risk: Some(4.9) },
                    matched: vec![MatchedAdvisory {
                        id: "GHSA-block".to_string(),
                        aliases: vec!["CVE-1".to_string()],
                        cvss: Some(9.8),
                        summary: Some("very bad".to_string()),
                    }],
                }],
            },
            PackageReport::unknown("mystery", "0.1.0", "baller"),
        ];

        let outcome = GateOutcome::new(reports, thresholds);
        assert_eq!(outcome.blocked().len(), 1);
        assert_eq!(outcome.blocked()[0].name, "blocked");
        assert_eq!(outcome.warned().len(), 1);
        assert_eq!(outcome.warned()[0].name, "warned");
        assert_eq!(outcome.unchecked().len(), 1);
        assert_eq!(outcome.unchecked()[0].name, "mystery");

        match outcome.block_error() {
            Some(BallError::RefereeBlocked { packages }) => {
                assert_eq!(packages.len(), 1);
                assert_eq!(packages[0].package, "blocked");
                assert_eq!(packages[0].advisories[0].id, "GHSA-block");
            }
            other => panic!("expected RefereeBlocked, got {:?}", other),
        }
    }

    #[test]
    fn test_gate_outcome_with_nothing_blocked_has_no_error() {
        let outcome = GateOutcome::new(Vec::new(), RefereeThresholds::default());
        assert!(outcome.block_error().is_none());
        assert!(outcome.to_json()["packages"].as_array().unwrap().is_empty());
        assert_eq!(outcome.to_json()["enabled"], true);
    }

    #[test]
    fn test_skipped_outcome_reports_as_disabled() {
        let outcome = GateOutcome::skipped(RefereeThresholds::default());
        assert!(outcome.skipped);
        assert_eq!(outcome.to_json()["enabled"], false);
        assert!(outcome.block_error().is_none());
        // Reporting a skipped gate must print nothing at all.
        outcome.report(false);
    }

    #[test]
    fn test_outcome_json_carries_the_thresholds() {
        let outcome = GateOutcome::new(Vec::new(), RefereeThresholds::default());
        let json = outcome.to_json();
        assert_eq!(json["warn_at"], 2.5);
        assert_eq!(json["block_at"], 4.0);
    }
}
