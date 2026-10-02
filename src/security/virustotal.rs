//! The optional VirusTotal hook.
//!
//! Enabled only by `referee.virustotal_api_key`, and deliberately **hash
//! only**: the SHA-256 of an executable is sent, never the file. Uploading a
//! user's binaries to a third party to find out whether they are safe would
//! give away more than the answer is worth, so Referee asks about hashes that
//! VirusTotal has already seen and accepts "no record" as an answer.
//!
//! Failure is always a non-answer, never a verdict. No key, a timeout, a rate
//! limit or an unknown hash all produce no finding — the offline scan's
//! conclusion stands on its own.

use std::path::Path;

use serde::Deserialize;

use crate::error::error::BallError;
use crate::http::HttpClient;
use crate::security::scan::{ScanFinding, ScanRule, ScanSeverity};
use crate::utils::security::sha256_file;

const VT_BASE: &str = "https://www.virustotal.com/api/v3";

/// Hashing every file in a large tree would cost more than the signal is
/// worth, so the hook looks at a bounded number of candidates.
const MAX_LOOKUPS: usize = 8;

#[derive(Debug, Deserialize)]
struct FileReport {
    data: ReportData,
}

#[derive(Debug, Deserialize)]
struct ReportData {
    #[serde(default)]
    attributes: ReportAttributes,
}

#[derive(Debug, Default, Deserialize)]
struct ReportAttributes {
    #[serde(default)]
    last_analysis_stats: AnalysisStats,
    #[serde(default)]
    meaningful_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct AnalysisStats {
    #[serde(default)]
    malicious: u32,
    #[serde(default)]
    suspicious: u32,
    #[serde(default)]
    #[allow(dead_code)]
    undetected: u32,
}

pub struct VirusTotalClient {
    http: HttpClient,
    api_key: String,
    base_url: String,
}

impl VirusTotalClient {
    pub fn new(http: HttpClient, api_key: String) -> Self {
        Self {
            http,
            api_key,
            base_url: VT_BASE.to_string(),
        }
    }

    /// Point the client at a different host, for tests and self-hosted proxies.
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self
    }

    /// Look up each candidate's hash and report the ones VirusTotal recognises.
    pub fn screen(&self, candidates: &[std::path::PathBuf], root: &Path) -> Vec<ScanFinding> {
        let mut findings = Vec::new();

        for path in candidates.iter().take(MAX_LOOKUPS) {
            let digest = match sha256_file(path) {
                Ok(digest) => digest,
                Err(e) => {
                    tracing::debug!("referee: cannot hash {}: {}", path.display(), e);
                    continue;
                }
            };

            match self.lookup(&digest) {
                Ok(Some(verdict)) => {
                    tracing::debug!(
                        "referee: virustotal reports {} malicious / {} suspicious for {}",
                        verdict.malicious,
                        verdict.suspicious,
                        path.display()
                    );
                    if verdict.malicious > 0 {
                        findings.push(ScanFinding {
                            path: path.strip_prefix(root).unwrap_or(path).to_path_buf(),
                            rule: ScanRule::VirusTotalDetection,
                            severity: ScanSeverity::Block,
                            evidence: format!(
                                "{} engine(s) flag sha256 {} as malicious{}",
                                verdict.malicious,
                                &digest[..16.min(digest.len())],
                                verdict
                                    .name
                                    .as_deref()
                                    .map(|name| format!(" ({})", name))
                                    .unwrap_or_default()
                            ),
                        });
                    }
                }
                Ok(None) => {
                    tracing::debug!("referee: virustotal has no record for {}", path.display());
                }
                Err(e) => {
                    // An unreachable third party never decides an install.
                    tracing::debug!("referee: virustotal lookup failed: {}", e);
                }
            }
        }

        findings
    }

    fn lookup(&self, sha256: &str) -> Result<Option<Verdict>, BallError> {
        let url = format!("{}/files/{}", self.base_url, sha256);
        let report: Option<FileReport> = self
            .http
            .get_json_optional_with_headers(&url, &[("x-apikey", self.api_key.as_str())])?;

        Ok(report.map(|report| Verdict {
            malicious: report.data.attributes.last_analysis_stats.malicious,
            suspicious: report.data.attributes.last_analysis_stats.suspicious,
            name: report.data.attributes.meaningful_name,
        }))
    }
}

struct Verdict {
    malicious: u32,
    suspicious: u32,
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_decodes_analysis_stats() {
        let raw = r#"{"data":{"attributes":{"last_analysis_stats":{"malicious":3,"suspicious":1,"undetected":60},"meaningful_name":"tool.exe"}}}"#;
        let report: FileReport = serde_json::from_str(raw).unwrap();
        assert_eq!(report.data.attributes.last_analysis_stats.malicious, 3);
        assert_eq!(report.data.attributes.last_analysis_stats.suspicious, 1);
        assert_eq!(report.data.attributes.last_analysis_stats.undetected, 60);
        assert_eq!(
            report.data.attributes.meaningful_name.as_deref(),
            Some("tool.exe")
        );
    }

    #[test]
    fn test_report_tolerates_missing_attributes() {
        let raw = r#"{"data":{}}"#;
        let report: FileReport = serde_json::from_str(raw).unwrap();
        assert_eq!(report.data.attributes.last_analysis_stats.malicious, 0);
        assert!(report.data.attributes.meaningful_name.is_none());
    }

    #[test]
    fn test_base_url_override_trims_its_slash() {
        let client = VirusTotalClient::new(HttpClient::new().unwrap(), "key".to_string())
            .with_base_url("https://vt.test/api/".to_string());
        assert_eq!(client.base_url, "https://vt.test/api");
    }

    #[test]
    fn test_screen_with_no_candidates_makes_no_requests() {
        let client = VirusTotalClient::new(HttpClient::new().unwrap(), "key".to_string())
            .with_base_url("http://127.0.0.1:1/never".to_string());
        assert!(client.screen(&[], Path::new("/")).is_empty());
    }
}
