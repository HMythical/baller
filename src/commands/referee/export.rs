//! Where `--format` output goes: stdout, or the `--out` file.
//!
//! The writers themselves live in `security::export`; this only picks one and
//! delivers what it rendered.

use std::path::Path;

use colored::Colorize;
use serde_json::Value;

use super::AuditFormat;
use crate::context::AppContext;
use crate::error::error::BallError;
use crate::security::export::{render_markdown, render_sarif, ScanOutcome};
use crate::security::GateOutcome;
use crate::utils::fs::atomic_write;

/// Everything an audit produced, for rendering.
pub struct AuditReport<'a> {
    pub ctx: &'a AppContext,
    pub outcome: &'a GateOutcome,
    pub scans: &'a [(String, ScanOutcome)],
    /// The `--json` document, reused verbatim by `--format json`
    pub document: &'a Value,
}

/// Render the audit in `format` and deliver it.
pub fn emit(format: AuditFormat, report: &AuditReport, out: Option<&str>) -> Result<(), BallError> {
    let rendered = match format {
        AuditFormat::Json => pretty(report.document)?,
        AuditFormat::Markdown => render_markdown(
            report.outcome,
            report.scans,
            report.ctx.referee.fail_policy().label(),
        ),
        AuditFormat::Sarif => pretty(&render_sarif(report.outcome, report.scans))?,
    };

    let label = match format {
        AuditFormat::Json => "JSON report",
        AuditFormat::Markdown => "Markdown report",
        AuditFormat::Sarif => "SARIF report",
    };
    deliver(&rendered, out, label)
}

/// Print `contents` on stdout, or write it to `out`.
pub fn deliver(contents: &str, out: Option<&str>, label: &str) -> Result<(), BallError> {
    match out {
        None => {
            print!("{}", with_newline(contents));
            Ok(())
        }
        Some(path) => {
            atomic_write(Path::new(path), with_newline(contents).as_bytes())?;
            tracing::info!("{} {} to {}", "Wrote".green().bold(), label, path);
            Ok(())
        }
    }
}

pub fn pretty(value: &Value) -> Result<String, BallError> {
    serde_json::to_string_pretty(value)
        .map_err(|e| BallError::InvalidConfig(format!("failed to render JSON output: {}", e)))
}

fn with_newline(contents: &str) -> String {
    if contents.ends_with('\n') {
        contents.to_string()
    } else {
        format!("{}\n", contents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deliver_writes_the_file_with_a_trailing_newline() {
        let dir =
            std::env::temp_dir().join(format!("baller_referee_export_{}", std::process::id()));
        let path = dir.join("nested").join("report.sarif");
        deliver("{}", Some(&path.to_string_lossy()), "SARIF").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_deliver_to_an_unwritable_path_is_an_error() {
        let dir =
            std::env::temp_dir().join(format!("baller_referee_export_file_{}", std::process::id()));
        std::fs::write(&dir, "a file, not a directory").unwrap();
        let path = dir.join("report.md");
        assert!(deliver("x", Some(&path.to_string_lossy()), "Markdown").is_err());
        let _ = std::fs::remove_file(&dir);
    }
}
