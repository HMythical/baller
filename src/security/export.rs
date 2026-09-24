//! Audit reports in formats other tools read: Markdown and SARIF 2.1.0.
//!
//! Both writers are pure: they take what an audit concluded — the Phase A
//! [`GateOutcome`] and the Phase B re-scan of each package — and return a
//! document. Where it goes (stdout or `--out FILE`) is the command's business.

use serde_json::{json, Value};

use crate::security::scan::{ScanFinding, ScanSeverity};
use crate::security::scoring::{classify, risk_index, Band};
use crate::security::verdict::PackageReport;
use crate::security::GateOutcome;

/// The SARIF 2.1.0 schema every document points at.
const SARIF_SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";

/// The rule a package nothing could be confirmed about is reported under.
const UNVERIFIED_RULE: &str = "referee/unverified";

/// What a re-scan of an installed package produced.
#[derive(Debug, Clone)]
pub enum ScanOutcome {
    /// No re-scan asked for, or a source that never produces an artifact
    Skipped,
    /// The recorded install path is no longer on disk
    Missing,
    Scanned(Vec<ScanFinding>),
}

impl ScanOutcome {
    /// The `scan` object `baller referee --json` gives each package.
    pub fn to_json(&self) -> Value {
        match self {
            ScanOutcome::Skipped => json!({ "scanned": false }),
            ScanOutcome::Missing => json!({ "scanned": false, "reason": "install path is gone" }),
            ScanOutcome::Scanned(findings) => json!({
                "scanned": true,
                "findings": findings.iter().map(ScanFinding::to_json).collect::<Vec<_>>(),
            }),
        }
    }
}

/// The re-scan recorded for a package, if one was run.
pub fn scan_for<'a>(scans: &'a [(String, ScanOutcome)], name: &str) -> Option<&'a ScanOutcome> {
    scans
        .iter()
        .find(|(scanned, _)| scanned == name)
        .map(|(_, outcome)| outcome)
}

/// The audit as a Markdown document: the same table the terminal shows, then
/// one section per package with anything to say about it.
pub fn render_markdown(
    outcome: &GateOutcome,
    scans: &[(String, ScanOutcome)],
    fail_policy: &str,
) -> String {
    let thresholds = &outcome.thresholds;
    let mut doc = String::from("# Referee audit\n\n");
    doc.push_str(&format!(
        "Warn at {:.2}, block at {:.2} on the 0-5 risk scale · fail policy `{}`\n\n",
        thresholds.warn_at, thresholds.block_at, fail_policy
    ));

    doc.push_str("| Package | Version | Status | Band | Risk | Advisories |\n");
    doc.push_str("|---|---|---|---|---|---|\n");
    for report in &outcome.reports {
        let advisories = report.advisories();
        let ids = if advisories.is_empty() {
            "—".to_string()
        } else {
            advisories
                .iter()
                .map(|advisory| advisory.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        doc.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            cell(&report.name),
            cell(&report.version),
            report.status().label(),
            report.band(thresholds).label(),
            risk_cell(report.risk()),
            cell(&ids)
        ));
    }

    let mut details = String::new();
    for report in &outcome.reports {
        let lines = detail_lines(report, scan_for(scans, &report.name));
        if lines.is_empty() {
            continue;
        }
        details.push_str(&format!("\n### {} {}\n\n", report.name, report.version));
        for line in lines {
            details.push_str(&format!("- {}\n", line));
        }
    }
    if !details.is_empty() {
        doc.push_str("\n## Details\n");
        doc.push_str(&details);
    }

    doc.push_str(&format!(
        "\n**Summary** {} package(s): {} over the block threshold, {} warned, {} not verified\n",
        outcome.reports.len(),
        outcome.blocked().len(),
        outcome.warned().len(),
        outcome.unchecked().len()
    ));
    doc
}

/// Everything the terminal report prints under a package's row.
fn detail_lines(report: &PackageReport, scan: Option<&ScanOutcome>) -> Vec<String> {
    let mut lines: Vec<String> = report
        .advisories()
        .iter()
        .map(|advisory| advisory.describe())
        .collect();

    for identity in report.unchecked() {
        lines.push(format!(
            "not verified: {} ({}) — {}",
            identity.identity.label(),
            identity.identity.scope.label(),
            identity.status.label()
        ));
    }

    match scan {
        Some(ScanOutcome::Missing) => {
            lines.push("artifact not on disk — nothing to re-scan".to_string())
        }
        Some(ScanOutcome::Scanned(findings)) => lines.extend(
            findings
                .iter()
                .map(|finding| format!("scan: {}", finding.describe())),
        ),
        Some(ScanOutcome::Skipped) | None => {}
    }

    lines
}

/// A Markdown table cell: pipes and newlines would break the row.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn risk_cell(risk: Option<f32>) -> String {
    match risk {
        Some(risk) => format!("{:.2}", risk),
        None => "—".to_string(),
    }
}

/// The audit as a SARIF 2.1.0 log.
///
/// One result per matched advisory, per scan finding, and per package that
/// could not be verified. Advisories are banded on their own CVSS against the
/// configured thresholds, so `block` maps to `error`, `warn` to `warning` and
/// `pass` to `note`; scan findings map `block`/`warn` the same way.
pub fn render_sarif(outcome: &GateOutcome, scans: &[(String, ScanOutcome)]) -> Value {
    let mut rules: Vec<Value> = Vec::new();
    let mut rule_ids: Vec<String> = Vec::new();
    let mut results: Vec<Value> = Vec::new();

    let mut add_rule = |id: String, rule: Value| -> usize {
        if let Some(index) = rule_ids.iter().position(|known| known == &id) {
            return index;
        }
        rule_ids.push(id);
        rules.push(rule);
        rules.len() - 1
    };

    for report in &outcome.reports {
        let properties = json!({
            "package": report.name,
            "version": report.version,
            "source": report.source,
            "status": report.status().label(),
        });

        for advisory in report.advisories() {
            let index = add_rule(
                advisory.id.clone(),
                json!({
                    "id": advisory.id,
                    "shortDescription": { "text": advisory.summary.as_deref().unwrap_or(&advisory.id) },
                    "helpUri": format!("https://osv.dev/vulnerability/{}", advisory.id),
                    "properties": { "aliases": advisory.aliases, "cvss": advisory.cvss },
                }),
            );
            let band = classify(advisory.cvss.map(risk_index), &outcome.thresholds);
            results.push(json!({
                "ruleId": advisory.id,
                "ruleIndex": index,
                "level": band_level(band),
                "message": { "text": format!("{} v{}: {}", report.name, report.version, advisory.describe()) },
                "locations": [package_location(report, None)],
                "properties": properties,
            }));
        }

        if report.status().is_unchecked() {
            let index = add_rule(
                UNVERIFIED_RULE.to_string(),
                json!({
                    "id": UNVERIFIED_RULE,
                    "shortDescription": { "text": "Advisory data could not be checked for this package" },
                }),
            );
            results.push(json!({
                "ruleId": UNVERIFIED_RULE,
                "ruleIndex": index,
                "level": "note",
                "message": { "text": format!("{} v{} was not verified: {}", report.name, report.version, report.status().label()) },
                "locations": [package_location(report, None)],
                "properties": properties,
            }));
        }

        if let Some(ScanOutcome::Scanned(findings)) = scan_for(scans, &report.name) {
            for finding in findings {
                let id = format!("scan/{}", finding.rule.label());
                let index = add_rule(
                    id.clone(),
                    json!({
                        "id": id,
                        "shortDescription": { "text": format!("Artifact scan rule '{}'", finding.rule.label()) },
                    }),
                );
                let level = match finding.severity {
                    ScanSeverity::Block => "error",
                    ScanSeverity::Warn => "warning",
                };
                results.push(json!({
                    "ruleId": id,
                    "ruleIndex": index,
                    "level": level,
                    "message": { "text": format!("{} v{}: {}", report.name, report.version, finding.describe()) },
                    "locations": [package_location(report, Some(finding))],
                    "properties": properties,
                }));
            }
        }
    }

    json!({
        "$schema": SARIF_SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "baller-referee",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/HMythical/baller",
                    "rules": rules,
                }
            },
            "results": results,
        }]
    })
}

/// The SARIF level a band maps to.
fn band_level(band: Band) -> &'static str {
    match band {
        Band::Block => "error",
        Band::Warn => "warning",
        Band::Pass => "note",
    }
}

/// Where a result points: the package, and the file inside it for a finding.
fn package_location(report: &PackageReport, finding: Option<&ScanFinding>) -> Value {
    let logical = json!([{ "name": report.name, "kind": "package" }]);
    match finding {
        Some(finding) => json!({
            "physicalLocation": {
                "artifactLocation": { "uri": finding.path.to_string_lossy().replace('\\', "/") }
            },
            "logicalLocations": logical,
        }),
        None => json!({ "logicalLocations": logical }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::identity::{AdvisoryIdentity, IdentityScope};
    use crate::security::scan::ScanRule;
    use crate::security::scoring::RefereeThresholds;
    use crate::security::verdict::{AdvisoryVerdict, MatchedAdvisory, Verdict};

    fn report(name: &str, status: Verdict, matched: Vec<MatchedAdvisory>) -> PackageReport {
        PackageReport {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            source: format!("cargo:{}", name),
            verdicts: vec![AdvisoryVerdict {
                identity: AdvisoryIdentity::new("crates.io", name, IdentityScope::Primary),
                status,
                matched,
            }],
        }
    }

    fn advisory(id: &str, cvss: Option<f32>) -> MatchedAdvisory {
        MatchedAdvisory {
            id: id.to_string(),
            aliases: vec![format!("CVE-{}", id)],
            cvss,
            summary: Some(format!("{} | flaw", id)),
        }
    }

    fn finding(severity: ScanSeverity) -> ScanFinding {
        ScanFinding {
            path: std::path::PathBuf::from("bin/install.sh"),
            rule: ScanRule::SuspiciousPayload,
            severity,
            evidence: "curl | sh".to_string(),
        }
    }

    /// alpha: critical advisory; beta: medium advisory + a warn finding;
    /// gamma: clean; delta: unverified.
    fn sample() -> (GateOutcome, Vec<(String, ScanOutcome)>) {
        let outcome = GateOutcome::new(
            vec![
                report(
                    "alpha",
                    Verdict::Vulnerable { risk: Some(4.9) },
                    vec![advisory("GHSA-crit", Some(9.8))],
                ),
                report(
                    "beta",
                    Verdict::Vulnerable { risk: Some(3.05) },
                    vec![advisory("GHSA-med", Some(6.1))],
                ),
                report("gamma", Verdict::Clean, Vec::new()),
                report("delta", Verdict::Unverified, Vec::new()),
            ],
            RefereeThresholds::default(),
        );
        let scans = vec![
            ("alpha".to_string(), ScanOutcome::Missing),
            (
                "beta".to_string(),
                ScanOutcome::Scanned(vec![finding(ScanSeverity::Warn)]),
            ),
            ("gamma".to_string(), ScanOutcome::Scanned(Vec::new())),
        ];
        (outcome, scans)
    }

    #[test]
    fn test_scan_outcome_json_keeps_the_audit_shape() {
        assert_eq!(ScanOutcome::Skipped.to_json(), json!({ "scanned": false }));
        assert_eq!(
            ScanOutcome::Missing.to_json()["reason"],
            json!("install path is gone")
        );
        let scanned = ScanOutcome::Scanned(vec![finding(ScanSeverity::Block)]).to_json();
        assert_eq!(scanned["scanned"], json!(true));
        assert_eq!(scanned["findings"][0]["rule"], json!("suspicious-payload"));
    }

    #[test]
    fn test_markdown_has_a_row_per_package_and_the_details() {
        let (outcome, scans) = sample();
        let doc = render_markdown(&outcome, &scans, "fail-open");

        assert!(doc.starts_with("# Referee audit"));
        assert!(doc.contains("fail policy `fail-open`"));
        assert!(doc.contains("| alpha | 1.0.0 | vulnerable | block | 4.90 | GHSA-crit |"));
        assert!(doc.contains("| beta | 1.0.0 | vulnerable | warn | 3.05 | GHSA-med |"));
        assert!(doc.contains("| gamma | 1.0.0 | clean | pass | — | — |"));
        assert!(doc.contains("| delta | 1.0.0 | unverified | pass | — | — |"));

        assert!(doc.contains("### alpha 1.0.0"));
        assert!(doc.contains("artifact not on disk — nothing to re-scan"));
        assert!(doc.contains("scan: warn [suspicious-payload]"));
        assert!(doc.contains("not verified: crates.io:delta (primary) — unverified"));
        // A clean, fully scanned package has nothing to detail.
        assert!(!doc.contains("### gamma"));
        assert!(doc.contains(
            "**Summary** 4 package(s): 1 over the block threshold, 1 warned, 1 not verified"
        ));
    }

    #[test]
    fn test_markdown_escapes_pipes_in_cells() {
        assert_eq!(cell("a|b\nc"), "a\\|b c");
    }

    #[test]
    fn test_sarif_shape_rules_and_results() {
        let (outcome, scans) = sample();
        let sarif = render_sarif(&outcome, &scans);

        assert_eq!(sarif["version"], json!("2.1.0"));
        assert_eq!(sarif["$schema"], json!(SARIF_SCHEMA));
        let run = &sarif["runs"][0];
        assert_eq!(run["tool"]["driver"]["name"], json!("baller-referee"));

        let rule_ids: Vec<&str> = run["tool"]["driver"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|rule| rule["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            rule_ids,
            vec![
                "GHSA-crit",
                "GHSA-med",
                "scan/suspicious-payload",
                UNVERIFIED_RULE
            ]
        );

        let results = run["results"].as_array().unwrap();
        let level_of = |rule: &str| -> (&str, &str) {
            let result = results
                .iter()
                .find(|result| result["ruleId"] == json!(rule))
                .unwrap();
            (
                result["level"].as_str().unwrap(),
                result["properties"]["package"].as_str().unwrap(),
            )
        };
        assert_eq!(results.len(), 4);
        assert_eq!(level_of("GHSA-crit"), ("error", "alpha"));
        assert_eq!(level_of("GHSA-med"), ("warning", "beta"));
        assert_eq!(level_of("scan/suspicious-payload"), ("warning", "beta"));
        assert_eq!(level_of(UNVERIFIED_RULE), ("note", "delta"));

        // Every result points at its rule by index, too.
        for result in results {
            let index = result["ruleIndex"].as_u64().unwrap() as usize;
            assert_eq!(rule_ids[index], result["ruleId"].as_str().unwrap());
        }

        let scan = results
            .iter()
            .find(|result| result["ruleId"] == json!("scan/suspicious-payload"))
            .unwrap();
        assert_eq!(
            scan["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
            json!("bin/install.sh")
        );
    }

    #[test]
    fn test_sarif_block_finding_is_an_error_and_rules_are_deduplicated() {
        let outcome = GateOutcome::new(
            vec![
                report(
                    "one",
                    Verdict::Vulnerable { risk: None },
                    vec![advisory("GHSA-shared", None)],
                ),
                report(
                    "two",
                    Verdict::Vulnerable { risk: None },
                    vec![advisory("GHSA-shared", None)],
                ),
            ],
            RefereeThresholds::default(),
        );
        let scans = vec![(
            "one".to_string(),
            ScanOutcome::Scanned(vec![
                finding(ScanSeverity::Block),
                finding(ScanSeverity::Warn),
            ]),
        )];
        let sarif = render_sarif(&outcome, &scans);
        let run = &sarif["runs"][0];

        assert_eq!(run["tool"]["driver"]["rules"].as_array().unwrap().len(), 2);
        let levels: Vec<&str> = run["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|result| result["level"].as_str().unwrap())
            .collect();
        // An unscored advisory warns, as the gate does.
        assert_eq!(levels, vec!["warning", "error", "warning", "warning"]);
    }

    #[test]
    fn test_sarif_for_a_clean_roster_has_no_results() {
        let outcome = GateOutcome::new(
            vec![report("gamma", Verdict::Clean, Vec::new())],
            RefereeThresholds::default(),
        );
        let sarif = render_sarif(&outcome, &[]);
        assert!(sarif["runs"][0]["results"].as_array().unwrap().is_empty());
        assert!(sarif["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap()
            .is_empty());
    }
}
