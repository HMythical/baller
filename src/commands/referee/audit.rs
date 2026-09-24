//! `baller referee audit` — Phase A and Phase B over the roster.
//!
//! Also what a bare `baller referee [PACKAGE]` runs.

use colored::Colorize;
use serde_json::{json, Value};

use super::export::{emit, AuditReport};
use super::{
    fail_on_error, print_table, report_empty, require_enabled, rescan, select_roster, AuditFormat,
    FailOn,
};
use crate::context::AppContext;
use crate::core::package::Package;
use crate::error::error::BallError;
use crate::security::export::{scan_for, ScanOutcome};
use crate::security::GateOutcome;
use crate::utils::output::print_json;

pub struct AuditOptions {
    /// Packages to audit; empty audits the whole roster
    pub package_names: Vec<String>,
    /// Ignore cached verdicts and re-query the advisory service
    pub refresh: bool,
    /// Check advisory data only; do not re-scan installed artifacts
    pub no_scan: bool,
    /// Exit non-zero when a package reaches this band
    pub fail_on: Option<FailOn>,
    /// Render the report in this format instead of the table
    pub format: Option<AuditFormat>,
    /// Write the formatted report to this file instead of stdout
    pub out: Option<String>,
}

pub fn execute_audit(ctx: &AppContext, opts: &AuditOptions) -> Result<(), BallError> {
    require_enabled(ctx)?;

    let installed = select_roster(ctx, &opts.package_names)?;

    let (outcome, scans) = if installed.is_empty() {
        // Nothing to ask about, but a CI job that asked for a report file still
        // gets a valid (empty) one.
        (
            GateOutcome::new(Vec::new(), *ctx.referee.thresholds()),
            Vec::new(),
        )
    } else {
        let packages: Vec<Package> = installed.iter().map(|row| row.to_package()).collect();

        tracing::info!(
            "{} {} package(s) — warn at {}, block at {} on the 0-5 risk scale",
            "Refereeing".green().bold(),
            packages.len(),
            format!("{:.2}", ctx.referee.thresholds().warn_at).yellow(),
            format!("{:.2}", ctx.referee.thresholds().block_at).yellow()
        );

        if opts.refresh {
            match ctx.db.referee_cache_clear() {
                Ok(dropped) => tracing::debug!("referee: dropped {} cached verdict(s)", dropped),
                Err(e) => tracing::debug!("referee: could not clear the cache: {}", e),
            }
        }

        match ctx.db.referee_cache_count() {
            Ok(count) => tracing::debug!("referee: {} cached verdict(s) available", count),
            Err(e) => tracing::debug!("referee: could not count the cache: {}", e),
        }

        let outcome = ctx.referee.audit(&ctx.db, &packages, opts.refresh)?;

        // The audit re-scans what is still on disk. A package whose extract
        // directory was swept is not a finding — there is simply nothing left
        // to look at, and that is reported as "not scanned" rather than "clean".
        let mut scans: Vec<(String, ScanOutcome)> = Vec::new();
        if !opts.no_scan {
            for (row, pkg) in installed.iter().zip(packages.iter()) {
                scans.push((pkg.name.clone(), rescan(ctx, pkg, &row.install_path)));
            }
        }
        (outcome, scans)
    };

    let document = audit_json(ctx, &outcome, &scans);

    if let Some(format) = opts.format {
        let report = AuditReport {
            ctx,
            outcome: &outcome,
            scans: &scans,
            document: &document,
        };
        emit(format, &report, opts.out.as_deref())?;
    }

    // A format with no file replaces the normal output on stdout; one written
    // to a file leaves stdout to the table (or `--json`) as usual.
    if opts.format.is_none() || opts.out.is_some() {
        print_default(ctx, installed.is_empty(), &outcome, &scans, &document)?;
    }

    match fail_on_error(&outcome, opts.fail_on) {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// The table, or the `--json` document.
fn print_default(
    ctx: &AppContext,
    empty: bool,
    outcome: &GateOutcome,
    scans: &[(String, ScanOutcome)],
    document: &Value,
) -> Result<(), BallError> {
    if empty {
        return report_empty(ctx);
    }
    if ctx.flags.json {
        return print_json(document);
    }
    print_table(ctx, &outcome.reports, scans);
    Ok(())
}

/// The single JSON document `baller referee --json` prints.
fn audit_json(ctx: &AppContext, outcome: &GateOutcome, scans: &[(String, ScanOutcome)]) -> Value {
    let entries: Vec<Value> = outcome
        .reports
        .iter()
        .map(|report| {
            let mut value = report.to_json();
            value["scan"] = scan_for(scans, &report.name)
                .unwrap_or(&ScanOutcome::Skipped)
                .to_json();
            value
        })
        .collect();

    json!({
        "command": "referee",
        "warn_at": ctx.referee.thresholds().warn_at,
        "block_at": ctx.referee.thresholds().block_at,
        "fail_policy": ctx.referee.fail_policy().label(),
        "packages": entries,
    })
}
