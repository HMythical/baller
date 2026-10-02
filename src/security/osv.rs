//! The OSV advisory API, as Referee's Phase A consumes it.
//!
//! One endpoint carries the gate: `POST /v1/querybatch` answers "which
//! advisories touch this (ecosystem, name, version)?" for a whole install plan
//! in a single round trip. `GET /v1/vulns/{id}` fills in the parts a batch
//! answer leaves out — severity, affected ranges, summary — and is only reached
//! for advisories that actually matched, so a clean plan costs exactly one
//! request.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::error::BallError;
use crate::http::HttpClient;

/// OSV's documented ceiling is 1000 queries per batch; baller stays well under
/// it so one oversized dependency tree cannot produce a request the service
/// rejects outright.
const MAX_BATCH: usize = 100;

/// One `(ecosystem, name, version)` lookup inside a batch.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OsvQuery {
    pub ecosystem: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
struct BatchRequest<'a> {
    queries: Vec<BatchQuery<'a>>,
}

#[derive(Debug, Serialize)]
struct BatchQuery<'a> {
    package: QueryPackage<'a>,
    version: &'a str,
}

#[derive(Debug, Serialize)]
struct QueryPackage<'a> {
    ecosystem: &'a str,
    name: &'a str,
}

#[derive(Debug, Deserialize)]
struct BatchResponse {
    #[serde(default)]
    results: Vec<BatchResult>,
}

#[derive(Debug, Deserialize)]
struct BatchResult {
    #[serde(default)]
    vulns: Vec<Vulnerability>,
}

/// An OSV vulnerability record.
///
/// Every field but `id` is optional on purpose: `querybatch` answers with only
/// `id` and `modified`, while `GET /v1/vulns/{id}` and a self-hosted or mocked
/// service return the whole record. The same type decodes both, and
/// [`Vulnerability::is_detailed`] reports which one arrived.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Vulnerability {
    pub id: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub severity: Vec<Severity>,
    #[serde(default)]
    pub affected: Vec<Affected>,
    #[serde(default)]
    pub database_specific: Option<Value>,
    /// Set when the advisory has been retracted; a withdrawn record must not
    /// influence a verdict.
    #[serde(default)]
    pub withdrawn: Option<String>,
}

impl Vulnerability {
    /// Whether this record carries enough to score and range-match it.
    ///
    /// A `querybatch` answer carries neither, so it has to be hydrated before
    /// it can produce a verdict.
    pub fn is_detailed(&self) -> bool {
        !self.affected.is_empty() || !self.severity.is_empty()
    }

    /// A withdrawn advisory is history, not a finding.
    pub fn is_withdrawn(&self) -> bool {
        self.withdrawn
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    }

    /// The one-line description shown next to the advisory id.
    pub fn short_summary(&self) -> Option<String> {
        let raw = self
            .summary
            .as_deref()
            .or(self.details.as_deref())?
            .split('\n')
            .find(|line| !line.trim().is_empty())?
            .trim();

        if raw.is_empty() {
            None
        } else {
            Some(crate::utils::fs::truncate_str(raw, 160))
        }
    }
}

/// A severity score attached to an advisory, as OSV encodes it.
///
/// `score` is a CVSS *vector* for the `CVSS_V2`/`CVSS_V3`/`CVSS_V4` types, and
/// some databases publish a bare number instead; [`crate::security::scoring`]
/// handles both.
#[derive(Debug, Clone, Deserialize)]
pub struct Severity {
    /// `CVSS_V2` / `CVSS_V3` / `CVSS_V4`; kept for wire fidelity — the scorer
    /// reads the vector's own prefix rather than trusting this label.
    #[allow(dead_code)]
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub score: String,
}

/// One `affected` entry: the package it is about, and the versions it covers.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Affected {
    #[serde(default)]
    pub package: Option<AffectedPackage>,
    #[serde(default)]
    pub ranges: Vec<Range>,
    #[serde(default)]
    pub versions: Vec<String>,
    #[serde(default)]
    pub severity: Vec<Severity>,
    /// Database-specific metadata; decoded so a record round-trips, not read.
    #[allow(dead_code)]
    #[serde(default)]
    pub database_specific: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AffectedPackage {
    #[serde(default)]
    pub ecosystem: String,
    #[serde(default)]
    pub name: String,
    /// Package URL; decoded so a record round-trips, not matched on.
    #[allow(dead_code)]
    #[serde(default)]
    pub purl: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Range {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub events: Vec<Event>,
}

/// A point on a range: where a vulnerability entered, and where it left.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Event {
    #[serde(default)]
    pub introduced: Option<String>,
    #[serde(default)]
    pub fixed: Option<String>,
    #[serde(default)]
    pub last_affected: Option<String>,
    #[serde(default)]
    pub limit: Option<String>,
}

pub struct OsvClient {
    http: HttpClient,
    base_url: String,
}

impl OsvClient {
    pub fn new(http: HttpClient, base_url: String) -> Self {
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Look up every query in one request per [`MAX_BATCH`] chunk.
    ///
    /// The returned vector is index-aligned with `queries`, so a caller can map
    /// results straight back onto the identities it asked about. A short
    /// response is padded with empty results rather than silently shifting
    /// every later answer onto the wrong package.
    pub fn query_batch(&self, queries: &[OsvQuery]) -> Result<Vec<Vec<Vulnerability>>, BallError> {
        let mut out = Vec::with_capacity(queries.len());

        for chunk in queries.chunks(MAX_BATCH) {
            let body = BatchRequest {
                queries: chunk
                    .iter()
                    .map(|query| BatchQuery {
                        package: QueryPackage {
                            ecosystem: &query.ecosystem,
                            name: &query.name,
                        },
                        version: &query.version,
                    })
                    .collect(),
            };

            let url = format!("{}/v1/querybatch", self.base_url);
            let response: BatchResponse = self.http.post_json(&url, &body)?;

            if response.results.len() != chunk.len() {
                tracing::debug!(
                    "osv: {} result(s) for {} quer(ies) — padding the remainder as unmatched",
                    response.results.len(),
                    chunk.len()
                );
            }

            let mut results = response.results;
            results.truncate(chunk.len());
            let missing = chunk.len() - results.len();
            out.extend(results.into_iter().map(|result| result.vulns));
            out.extend(std::iter::repeat_with(Vec::new).take(missing));
        }

        Ok(out)
    }

    /// Fetch one advisory in full.
    ///
    /// `Ok(None)` means the service has no record under that id — a real answer
    /// from OSV, not a failure, so it must not abort a gate that is otherwise
    /// complete.
    pub fn vuln(&self, id: &str) -> Result<Option<Vulnerability>, BallError> {
        let url = format!("{}/v1/vulns/{}", self.base_url, id);
        self.http.get_json_optional_with_headers(&url, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_result_decodes_id_only_vulns() {
        let raw =
            r#"{"results":[{"vulns":[{"id":"GHSA-aaaa","modified":"2024-01-01T00:00:00Z"}]},{}]}"#;
        let decoded: BatchResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(decoded.results.len(), 2);
        assert_eq!(decoded.results[0].vulns[0].id, "GHSA-aaaa");
        assert!(!decoded.results[0].vulns[0].is_detailed());
        assert!(decoded.results[1].vulns.is_empty());
    }

    #[test]
    fn test_full_record_decodes_and_is_detailed() {
        let raw = r#"{
            "id": "RUSTSEC-2021-0001",
            "summary": "a flaw\nsecond line",
            "aliases": ["CVE-2021-1111"],
            "severity": [{"type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"}],
            "affected": [{
                "package": {"ecosystem": "crates.io", "name": "serde"},
                "ranges": [{"type": "SEMVER", "events": [{"introduced": "1.0.0"}, {"fixed": "1.0.5"}]}],
                "versions": ["1.0.1"]
            }]
        }"#;
        let vuln: Vulnerability = serde_json::from_str(raw).unwrap();
        assert!(vuln.is_detailed());
        assert!(!vuln.is_withdrawn());
        assert_eq!(vuln.aliases, vec!["CVE-2021-1111".to_string()]);
        assert_eq!(vuln.short_summary().unwrap(), "a flaw");
        assert_eq!(vuln.affected[0].ranges[0].events.len(), 2);
    }

    #[test]
    fn test_withdrawn_is_detected() {
        let raw = r#"{"id":"X","withdrawn":"2024-02-02T00:00:00Z"}"#;
        let vuln: Vulnerability = serde_json::from_str(raw).unwrap();
        assert!(vuln.is_withdrawn());
    }

    #[test]
    fn test_blank_withdrawn_is_not_withdrawn() {
        let raw = r#"{"id":"X","withdrawn":"   "}"#;
        let vuln: Vulnerability = serde_json::from_str(raw).unwrap();
        assert!(!vuln.is_withdrawn());
    }

    #[test]
    fn test_short_summary_falls_back_to_details() {
        let raw = r#"{"id":"X","details":"\n\nthe long story"}"#;
        let vuln: Vulnerability = serde_json::from_str(raw).unwrap();
        assert_eq!(vuln.short_summary().unwrap(), "the long story");
    }

    #[test]
    fn test_short_summary_is_truncated() {
        let long = "x".repeat(400);
        let vuln = Vulnerability {
            id: "X".to_string(),
            summary: Some(long),
            ..Default::default()
        };
        let summary = vuln.short_summary().unwrap();
        assert_eq!(summary.chars().count(), 160);
        assert!(summary.ends_with("..."));
    }

    #[test]
    fn test_batch_request_serializes_to_the_documented_shape() {
        let body = BatchRequest {
            queries: vec![BatchQuery {
                package: QueryPackage {
                    ecosystem: "crates.io",
                    name: "serde",
                },
                version: "1.0.229",
            }],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["queries"][0]["package"]["ecosystem"], "crates.io");
        assert_eq!(json["queries"][0]["package"]["name"], "serde");
        assert_eq!(json["queries"][0]["version"], "1.0.229");
    }

    #[test]
    fn test_base_url_loses_its_trailing_slash() {
        let client = OsvClient::new(HttpClient::new().unwrap(), "https://osv.test/".to_string());
        assert_eq!(client.base_url(), "https://osv.test");
    }
}
