//! `baller referee scan` — artifact only (Phase B).
//!
//! Re-scans each installed package's extracted tree without consulting any
//! advisory data. A swept extract directory is reported as not on disk, which
//! is distinct from a clean scan.

use colored::Colorize;
use serde_json::{json, Value};

use super::{
    print_scan_lines, report_empty, require_enabled, rescan, scan_fail_on_error, select_roster,
    FailOn,
};
use crate::commands::draft::source_label;
use crate::context::AppContext;
use crate::core::package::Package;
use crate::error::error::BallError;
use crate::security::export::ScanOutcome;
use crate::security::scan::has_blocking;
use crate::utils::output::print_json;

pub fn execute_scan(
    ctx: &AppContext,
    package_names: &[String],
    fail_on: Option<FailOn>,
) -> Result<(), BallError> {
    require_enabled(ctx)?;

    let installed = select_roster(ctx, package_names)?;
    if installed.is_empty() {
        return report_empty(ctx);
    }

    tracing::info!(
        "{} {} package(s)",
        "Scanning".green().bold(),
        installed.len()
    );

    let results: Vec<_> = installed
        .iter()
        .map(|row| {
            let pkg = row.to_package();
            let outcome = rescan(ctx, &pkg, &row.install_path);
            (pkg, outcome)
        })
        .collect();

    if ctx.flags.json {
        let packages: Vec<Value> = results
            .iter()
            .map(|(pkg, outcome)| {
                json!({
                    "name": pkg.name,
                    "version": pkg.version,
                    "source": source_label(&pkg.source),
                    "scan": outcome.to_json(),
                })
            })
            .collect();
        print_json(&json!({
            "command": "referee",
            "subcommand": "scan",
            "packages": packages,
        }))?;
        return fail_result(&results, fail_on);
    }

    println!(
        "{:<24} {:<14} {}",
        "PACKAGE".bold(),
        "VERSION".bold(),
        "SCAN".bold()
    );

    let (mut blocking, mut warned, mut unscanned) = (0, 0, 0);
    for (pkg, outcome) in &results {
        let status = match outcome {
            ScanOutcome::Skipped => {
                unscanned += 1;
                "not scanned".normal()
            }
            ScanOutcome::Missing => {
                unscanned += 1;
                "not on disk".yellow()
            }
            ScanOutcome::Scanned(findings) if has_blocking(findings) => {
                blocking += 1;
                format!("{} finding(s)", findings.len()).red().bold()
            }
            ScanOutcome::Scanned(findings) if !findings.is_empty() => {
                warned += 1;
                format!("{} finding(s)", findings.len()).yellow().bold()
            }
            ScanOutcome::Scanned(_) => "clean".green(),
        };

        println!(
            "{:<24} {:<14} {}",
            pkg.name.cyan(),
            pkg.version.yellow(),
            status
        );
        print_scan_lines(outcome);
    }

    println!();
    println!(
        "{} {} package(s): {} with blocking findings, {} with warnings, {} not scanned",
        "Summary".bold(),
        results.len(),
        blocking,
        warned,
        unscanned
    );
    println!(
        "{} the scan changes nothing — eject or update a flagged package yourself",
        "Note".yellow()
    );
    fail_result(&results, fail_on)
}

/// The report is printed either way; `--fail-on` only decides the exit code.
fn fail_result(
    results: &[(Package, ScanOutcome)],
    fail_on: Option<FailOn>,
) -> Result<(), BallError> {
    match scan_fail_on_error(results, fail_on) {
        Some(err) => Err(err),
        None => Ok(()),
    }
}
