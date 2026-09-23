//! What Referee concluded, and how it says so.
//!
//! A verdict is deliberately four-valued rather than a boolean. "We asked and
//! found nothing" (`Clean`) and "we could not ask" (`Unknown`/`Unverified`) are
//! different facts, and collapsing them would let an outage read as a clean
//! bill of health. Every report keeps them apart, and the gate prints the
//! unverified ones so the user can see what was *not* checked.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::db::CachedVerdict;
use crate::security::identity::AdvisoryIdentity;
use crate::security::scoring::{classify, Band, RefereeThresholds};

/// One advisory that matched the version being installed.
///
/// Serialisable because this is exactly what the verdict cache stores: an
/// advisory list is the part of a verdict a later run cannot recompute without
/// going back to the network.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchedAdvisory {
    /// The advisory id, e.g. `GHSA-xxxx-yyyy-zzzz` or `RUSTSEC-2021-0001`
    pub id: String,
    /// Other ids the same issue is tracked under, e.g. `CVE-2021-1111`
    pub aliases: Vec<String>,
    /// The highest CVSS base score published for it, when there is one
    pub cvss: Option<f32>,
    /// A one-line description, when the record carries one
    pub summary: Option<String>,
}

impl MatchedAdvisory {
    /// `GHSA-xxxx (CVE-2021-1111) CVSS 7.5 — summary`, for a report line.
    pub fn describe(&self) -> String {
        let mut line = self.id.clone();
        if !self.aliases.is_empty() {
            line.push_str(&format!(" ({})", self.aliases.join(", ")));
        }
        match self.cvss {
            Some(cvss) => line.push_str(&format!(" CVSS {:.1}", cvss)),
            None => line.push_str(" CVSS unscored"),
        }
        if let Some(summary) = self.summary.as_deref() {
            line.push_str(&format!(" — {}", summary));
        }
        line
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "aliases": self.aliases,
            "cvss": round_to(self.cvss, 1),
            "summary": self.summary,
        })
    }
}

/// The outcome of checking one identity, or of aggregating a package's set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    /// Queried, and no advisory matched this version
    Clean,
    /// An advisory matched; `risk` is `None` when none of them carried a score
    Vulnerable { risk: Option<f32> },
    /// No ecosystem to query, or the advisory data could not be read
    Unknown,
    /// The advisory service was unreachable; the fail-open path
    Unverified,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Clean => "clean",
            Verdict::Vulnerable { .. } => "vulnerable",
            Verdict::Unknown => "unknown",
            Verdict::Unverified => "unverified",
        }
    }

    pub fn risk(self) -> Option<f32> {
        match self {
            Verdict::Vulnerable { risk } => risk,
            _ => None,
        }
    }

    pub fn is_vulnerable(self) -> bool {
        matches!(self, Verdict::Vulnerable { .. })
    }

    /// Whether this verdict means "nobody actually checked".
    ///
    /// `Unknown` and `Unverified` differ in cause but not in consequence: the
    /// user is told, and neither is ever presented as safe.
    pub fn is_unchecked(self) -> bool {
        matches!(self, Verdict::Unknown | Verdict::Unverified)
    }
}

impl CachedVerdict {
    /// Rebuild the verdict this cache row recorded.
    ///
    /// A row whose `verdict` column holds something this build does not
    /// recognise — an older or newer baller wrote it — becomes `Unknown`
    /// rather than a guess, and the identity is re-queried the next time the
    /// cache is written.
    pub fn into_verdict(self, identity: AdvisoryIdentity) -> AdvisoryVerdict {
        let matched: Vec<MatchedAdvisory> =
            serde_json::from_str(&self.advisories).unwrap_or_default();

        let status = match self.verdict.as_str() {
            "clean" => Verdict::Clean,
            "vulnerable" => Verdict::Vulnerable {
                risk: self.risk.map(|risk| risk as f32),
            },
            other => {
                tracing::debug!("referee: cached verdict '{}' is not recognised", other);
                Verdict::Unknown
            }
        };

        AdvisoryVerdict {
            identity,
            status,
            matched,
        }
    }
}

/// Render a score for JSON without its binary-float tail.
///
/// An `f32` CVSS of 9.8 widens to 9.800000190734863 on the way into JSON, which
/// is noise in a document a person reads and a script compares. Scores are
/// published to one decimal and risk indices are halves of them, so rounding at
/// the boundary loses nothing real.
fn round_to(value: Option<f32>, places: u32) -> Value {
    match value {
        Some(value) => {
            let factor = 10f64.powi(places as i32);
            json!(((value as f64) * factor).round() / factor)
        }
        None => Value::Null,
    }
}

/// The result of checking one identity.
#[derive(Debug, Clone)]
pub struct AdvisoryVerdict {
    pub identity: AdvisoryIdentity,
    pub status: Verdict,
    pub matched: Vec<MatchedAdvisory>,
}

impl AdvisoryVerdict {
    pub fn to_json(&self) -> Value {
        json!({
            "ecosystem": self.identity.ecosystem,
            "name": self.identity.name,
            "scope": self.identity.scope.label(),
            "status": self.status.label(),
            "risk": round_to(self.status.risk(), 2),
            "advisories": self.matched.iter().map(MatchedAdvisory::to_json).collect::<Vec<_>>(),
        })
    }
}

/// Everything Referee concluded about one package in a plan.
#[derive(Debug, Clone)]
pub struct PackageReport {
    pub name: String,
    pub version: String,
    /// The source label, as `draft` spells it (`github:owner/repo`, `cargo:x`)
    pub source: String,
    pub verdicts: Vec<AdvisoryVerdict>,
}

impl PackageReport {
    /// A package nothing could be asked about.
    pub fn unknown(name: &str, version: &str, source: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            source: source.to_string(),
            verdicts: Vec::new(),
        }
    }

    /// The package's overall status: the worst news any identity returned.
    ///
    /// A vulnerability found under any identity is a vulnerability in what gets
    /// installed, so it wins outright. Below that, an outage outranks a missing
    /// mapping, and `Clean` requires at least one identity to have actually
    /// answered.
    pub fn status(&self) -> Verdict {
        if self.verdicts.is_empty() {
            return Verdict::Unknown;
        }

        let risks: Vec<Option<f32>> = self
            .verdicts
            .iter()
            .filter(|verdict| verdict.status.is_vulnerable())
            .map(|verdict| verdict.status.risk())
            .collect();

        if !risks.is_empty() {
            // `None` here is "matched but unscored"; a real number outranks it
            // because it is the one that can cross a threshold.
            let risk = risks
                .iter()
                .copied()
                .flatten()
                .fold(None, |best: Option<f32>, risk| {
                    Some(best.map_or(risk, |best: f32| best.max(risk)))
                });
            return Verdict::Vulnerable { risk };
        }

        if self
            .verdicts
            .iter()
            .any(|verdict| verdict.status == Verdict::Unverified)
        {
            return Verdict::Unverified;
        }

        if self
            .verdicts
            .iter()
            .all(|verdict| verdict.status == Verdict::Unknown)
        {
            return Verdict::Unknown;
        }

        Verdict::Clean
    }

    /// The risk index the thresholds are applied to.
    pub fn risk(&self) -> Option<f32> {
        self.status().risk()
    }

    /// Every advisory that matched, across all identities, worst first.
    pub fn advisories(&self) -> Vec<&MatchedAdvisory> {
        let mut all: Vec<&MatchedAdvisory> = self
            .verdicts
            .iter()
            .flat_map(|verdict| verdict.matched.iter())
            .collect();
        all.sort_by(|a, b| {
            b.cvss
                .unwrap_or(-1.0)
                .partial_cmp(&a.cvss.unwrap_or(-1.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all
    }

    /// What the gate does about this package.
    ///
    /// Only a matched advisory can be banded: a package nothing was found for
    /// is reported, never blocked, however strict the thresholds are.
    pub fn band(&self, thresholds: &RefereeThresholds) -> Band {
        match self.status() {
            Verdict::Vulnerable { risk } => classify(risk, thresholds),
            _ => Band::Pass,
        }
    }

    /// The identities that could not be checked, for the "not verified" line.
    pub fn unchecked(&self) -> Vec<&AdvisoryVerdict> {
        self.verdicts
            .iter()
            .filter(|verdict| verdict.status.is_unchecked())
            .collect()
    }

    /// One line explaining why this package was blocked.
    pub fn block_reason(&self, thresholds: &RefereeThresholds) -> String {
        match self.risk() {
            Some(risk) => format!(
                "risk index {:.2} is at or above the block threshold of {:.2}",
                risk, thresholds.block_at
            ),
            None => "a matched advisory crossed the block threshold".to_string(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "version": self.version,
            "source": self.source,
            "status": self.status().label(),
            "band": self.band(&RefereeThresholds::default()).label(),
            "risk": round_to(self.risk(), 2),
            "advisories": self.advisories().iter().map(|a| a.to_json()).collect::<Vec<_>>(),
            "identities": self.verdicts.iter().map(AdvisoryVerdict::to_json).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::identity::{AdvisoryIdentity, IdentityScope};

    fn identity(name: &str) -> AdvisoryIdentity {
        AdvisoryIdentity::new("crates.io", name, IdentityScope::Primary)
    }

    fn advisory(id: &str, cvss: Option<f32>) -> MatchedAdvisory {
        MatchedAdvisory {
            id: id.to_string(),
            aliases: vec!["CVE-2021-1111".to_string()],
            cvss,
            summary: Some("a flaw".to_string()),
        }
    }

    fn report(verdicts: Vec<AdvisoryVerdict>) -> PackageReport {
        PackageReport {
            name: "tool".to_string(),
            version: "1.0.0".to_string(),
            source: "cargo:tool".to_string(),
            verdicts,
        }
    }

    fn verdict(status: Verdict, matched: Vec<MatchedAdvisory>) -> AdvisoryVerdict {
        AdvisoryVerdict {
            identity: identity("tool"),
            status,
            matched,
        }
    }

    #[test]
    fn test_no_identities_is_unknown() {
        let report = PackageReport::unknown("tool", "1.0.0", "baller");
        assert_eq!(report.status(), Verdict::Unknown);
        assert_eq!(report.risk(), None);
        assert_eq!(report.band(&RefereeThresholds::default()), Band::Pass);
    }

    #[test]
    fn test_clean_needs_one_identity_to_have_answered() {
        let report = report(vec![verdict(Verdict::Clean, Vec::new())]);
        assert_eq!(report.status(), Verdict::Clean);
    }

    #[test]
    fn test_all_unknown_identities_stay_unknown() {
        let report = report(vec![
            verdict(Verdict::Unknown, Vec::new()),
            verdict(Verdict::Unknown, Vec::new()),
        ]);
        assert_eq!(report.status(), Verdict::Unknown);
    }

    #[test]
    fn test_one_clean_answer_outranks_an_unknown_identity() {
        let report = report(vec![
            verdict(Verdict::Clean, Vec::new()),
            verdict(Verdict::Unknown, Vec::new()),
        ]);
        assert_eq!(report.status(), Verdict::Clean);
    }

    #[test]
    fn test_an_outage_outranks_a_clean_answer() {
        let report = report(vec![
            verdict(Verdict::Clean, Vec::new()),
            verdict(Verdict::Unverified, Vec::new()),
        ]);
        assert_eq!(report.status(), Verdict::Unverified);
    }

    #[test]
    fn test_a_vulnerability_under_any_identity_wins() {
        let report = report(vec![
            verdict(Verdict::Clean, Vec::new()),
            verdict(Verdict::Unverified, Vec::new()),
            verdict(
                Verdict::Vulnerable { risk: Some(3.75) },
                vec![advisory("GHSA-a", Some(7.5))],
            ),
        ]);
        assert_eq!(report.status(), Verdict::Vulnerable { risk: Some(3.75) });
    }

    #[test]
    fn test_risk_is_the_maximum_across_identities() {
        let report = report(vec![
            verdict(
                Verdict::Vulnerable { risk: Some(1.5) },
                vec![advisory("GHSA-a", Some(3.0))],
            ),
            verdict(
                Verdict::Vulnerable { risk: Some(4.5) },
                vec![advisory("GHSA-b", Some(9.0))],
            ),
        ]);
        assert_eq!(report.risk(), Some(4.5));
        assert_eq!(report.band(&RefereeThresholds::default()), Band::Block);
    }

    #[test]
    fn test_a_scored_identity_outranks_an_unscored_one() {
        let report = report(vec![
            verdict(
                Verdict::Vulnerable { risk: None },
                vec![advisory("GHSA-a", None)],
            ),
            verdict(
                Verdict::Vulnerable { risk: Some(1.0) },
                vec![advisory("GHSA-b", Some(2.0))],
            ),
        ]);
        assert_eq!(report.risk(), Some(1.0));
        assert_eq!(report.band(&RefereeThresholds::default()), Band::Pass);
    }

    #[test]
    fn test_an_entirely_unscored_match_warns() {
        let report = report(vec![verdict(
            Verdict::Vulnerable { risk: None },
            vec![advisory("GHSA-a", None)],
        )]);
        assert_eq!(report.risk(), None);
        assert_eq!(report.band(&RefereeThresholds::default()), Band::Warn);
    }

    #[test]
    fn test_advisories_are_listed_worst_first() {
        let report = report(vec![verdict(
            Verdict::Vulnerable { risk: Some(4.5) },
            vec![
                advisory("low", Some(2.0)),
                advisory("unscored", None),
                advisory("high", Some(9.0)),
            ],
        )]);
        let ids: Vec<&str> = report.advisories().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["high", "low", "unscored"]);
    }

    #[test]
    fn test_unchecked_lists_both_unknown_and_unverified() {
        let report = report(vec![
            verdict(Verdict::Clean, Vec::new()),
            verdict(Verdict::Unknown, Vec::new()),
            verdict(Verdict::Unverified, Vec::new()),
        ]);
        assert_eq!(report.unchecked().len(), 2);
    }

    #[test]
    fn test_describe_renders_id_aliases_score_and_summary() {
        assert_eq!(
            advisory("GHSA-a", Some(7.5)).describe(),
            "GHSA-a (CVE-2021-1111) CVSS 7.5 — a flaw"
        );
    }

    #[test]
    fn test_describe_says_unscored_rather_than_implying_zero() {
        assert!(advisory("GHSA-a", None)
            .describe()
            .contains("CVSS unscored"));
    }

    #[test]
    fn test_describe_omits_an_empty_alias_list() {
        let advisory = MatchedAdvisory {
            id: "GHSA-a".to_string(),
            aliases: Vec::new(),
            cvss: Some(1.0),
            summary: None,
        };
        assert_eq!(advisory.describe(), "GHSA-a CVSS 1.0");
    }

    #[test]
    fn test_report_json_matches_the_documented_shape() {
        let report = report(vec![verdict(
            Verdict::Vulnerable { risk: Some(3.75) },
            vec![advisory("GHSA-a", Some(7.5))],
        )]);
        let json = report.to_json();
        assert_eq!(json["name"], "tool");
        assert_eq!(json["version"], "1.0.0");
        assert_eq!(json["source"], "cargo:tool");
        assert_eq!(json["status"], "vulnerable");
        assert_eq!(json["risk"], 3.75);
        assert_eq!(json["advisories"][0]["id"], "GHSA-a");
        assert_eq!(json["advisories"][0]["aliases"][0], "CVE-2021-1111");
        assert_eq!(json["advisories"][0]["cvss"], 7.5);
        assert_eq!(json["identities"][0]["scope"], "primary");
    }

    #[test]
    fn test_json_scores_are_rounded_to_their_published_precision() {
        let report = report(vec![verdict(
            Verdict::Vulnerable { risk: Some(4.9) },
            vec![advisory("GHSA-a", Some(9.8))],
        )]);
        let rendered = serde_json::to_string(&report.to_json()).unwrap();
        assert!(rendered.contains("\"cvss\":9.8"), "{}", rendered);
        assert!(rendered.contains("\"risk\":4.9"), "{}", rendered);
        assert!(!rendered.contains("9.80000"), "{}", rendered);
    }

    #[test]
    fn test_round_to_passes_none_through_as_null() {
        assert!(round_to(None, 2).is_null());
        assert_eq!(round_to(Some(3.0499), 2), json!(3.05));
    }

    #[test]
    fn test_unknown_report_json_has_a_null_risk() {
        let json = PackageReport::unknown("tool", "1.0.0", "baller").to_json();
        assert_eq!(json["status"], "unknown");
        assert!(json["risk"].is_null());
    }

    #[test]
    fn test_block_reason_names_the_threshold() {
        let report = report(vec![verdict(
            Verdict::Vulnerable { risk: Some(4.5) },
            vec![advisory("GHSA-a", Some(9.0))],
        )]);
        let reason = report.block_reason(&RefereeThresholds::default());
        assert!(reason.contains("4.50"));
        assert!(reason.contains("4.00"));
    }

    #[test]
    fn test_cached_clean_verdict_round_trips() {
        let cached = CachedVerdict {
            verdict: "clean".to_string(),
            risk: None,
            advisories: "[]".to_string(),
            checked_at: "2026-01-01 00:00:00".to_string(),
        };
        let restored = cached.into_verdict(identity("tool"));
        assert_eq!(restored.status, Verdict::Clean);
        assert!(restored.matched.is_empty());
    }

    #[test]
    fn test_cached_vulnerable_verdict_round_trips() {
        let advisories = serde_json::to_string(&vec![advisory("GHSA-a", Some(7.5))]).unwrap();
        let cached = CachedVerdict {
            verdict: "vulnerable".to_string(),
            risk: Some(3.75),
            advisories,
            checked_at: "2026-01-01 00:00:00".to_string(),
        };
        let restored = cached.into_verdict(identity("tool"));
        assert_eq!(restored.status, Verdict::Vulnerable { risk: Some(3.75) });
        assert_eq!(restored.matched[0].id, "GHSA-a");
        assert_eq!(restored.matched[0].cvss, Some(7.5));
    }

    #[test]
    fn test_unrecognised_cached_verdict_becomes_unknown() {
        let cached = CachedVerdict {
            verdict: "quantum".to_string(),
            risk: Some(1.0),
            advisories: "[]".to_string(),
            checked_at: String::new(),
        };
        assert_eq!(
            cached.into_verdict(identity("tool")).status,
            Verdict::Unknown
        );
    }

    #[test]
    fn test_unreadable_cached_advisories_do_not_panic() {
        let cached = CachedVerdict {
            verdict: "vulnerable".to_string(),
            risk: Some(2.0),
            advisories: "not json".to_string(),
            checked_at: String::new(),
        };
        let restored = cached.into_verdict(identity("tool"));
        assert_eq!(restored.status, Verdict::Vulnerable { risk: Some(2.0) });
        assert!(restored.matched.is_empty());
    }

    #[test]
    fn test_verdict_labels_and_predicates() {
        assert_eq!(Verdict::Clean.label(), "clean");
        assert_eq!(Verdict::Unknown.label(), "unknown");
        assert_eq!(Verdict::Unverified.label(), "unverified");
        assert_eq!(
            Verdict::Vulnerable { risk: Some(1.0) }.label(),
            "vulnerable"
        );
        assert!(Verdict::Unknown.is_unchecked());
        assert!(Verdict::Unverified.is_unchecked());
        assert!(!Verdict::Clean.is_unchecked());
        assert!(Verdict::Vulnerable { risk: None }.is_vulnerable());
    }
}
