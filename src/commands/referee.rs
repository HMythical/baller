//! `baller referee` — audit the roster against public vulnerability data.
//!
//! The audit is deliberately **read-only**. A package that would be blocked on
//! install is still on the roster because it was installed before the advisory
//! existed, or before Referee did; silently ejecting it would be a far worse
//! surprise than reporting it. The command's job is to tell the user what they
//! are running, and let them decide.

use colored::Colorize;
use serde_json::json;

#[cfg(test)]
use crate::commands::draft::source_label;
use crate::context::AppContext;
use crate::core::db::InstalledPackage;
use crate::core::package::Package;
use crate::error::error::BallError;
use crate::security::scan::{ScanFinding, ScanSeverity};
use crate::security::scoring::Band;
use crate::security::verdict::{PackageReport, Verdict};
use crate::utils::output::print_json;

pub struct RefereeOptions {
    /// Ignore cached verdicts and re-query the advisory service
    pub refresh: bool,
    /// Check advisory data only; do not re-scan installed artifacts
    pub no_scan: bool,
}

pub fn execute_referee(
    ctx: &AppContext,
    package_name: Option<&str>,
    opts: &RefereeOptions,
) -> Result<(), BallError> {
    if !ctx.referee.enabled() {
        return Err(BallError::InvalidConfig(
            "referee is disabled — remove --no-referee, or set 'enabled = true' under [referee] in baller.conf"
                .to_string(),
        ));
    }

    let installed = match package_name {
        Some(name) => vec![resolve_roster_entry(ctx, name)?],
        None => ctx.db.list_packages()?,
    };

    if installed.is_empty() {
        return report_empty(ctx, package_name);
    }

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
    // directory was swept is not a finding — there is simply nothing left to
    // look at, and that is reported as "not scanned" rather than "clean".
    let mut scans: Vec<(String, ScanOutcome)> = Vec::new();
    if !opts.no_scan {
        for (row, pkg) in installed.iter().zip(packages.iter()) {
            scans.push((pkg.name.clone(), rescan(ctx, pkg, &row.install_path)));
        }
    }

    if ctx.flags.json {
        let entries: Vec<_> = outcome
            .reports
            .iter()
            .map(|report| {
                let scan = scans
                    .iter()
                    .find(|(name, _)| name == &report.name)
                    .map(|(_, outcome)| outcome);
                let mut value = report.to_json();
                value["scan"] = match scan {
                    Some(ScanOutcome::Skipped) | None => json!({ "scanned": false }),
                    Some(ScanOutcome::Missing) => {
                        json!({ "scanned": false, "reason": "install path is gone" })
                    }
                    Some(ScanOutcome::Scanned(findings)) => json!({
                        "scanned": true,
                        "findings": findings.iter().map(ScanFinding::to_json).collect::<Vec<_>>(),
                    }),
                };
                value
            })
            .collect();

        return print_json(&json!({
            "command": "referee",
            "warn_at": ctx.referee.thresholds().warn_at,
            "block_at": ctx.referee.thresholds().block_at,
            "fail_policy": ctx.referee.fail_policy().label(),
            "packages": entries,
        }));
    }

    print_table(ctx, &outcome.reports, &scans);
    Ok(())
}

/// Find a roster entry by the name the user typed.
///
/// Packages are rostered under their bare name, but people refer to them the
/// way they installed them — `BurntSushi/ripgrep`, `cargo:ripgrep`, or just
/// `ripgrep`. The exact name is tried first, then the last path segment, then
/// the part after a source prefix, so all three forms find the same row.
fn resolve_roster_entry(ctx: &AppContext, name: &str) -> Result<InstalledPackage, BallError> {
    let trimmed = name.trim();

    let mut candidates = vec![trimmed.to_string()];
    if let Some((_, tail)) = trimmed.rsplit_once('/') {
        candidates.push(tail.to_string());
    }
    if let Some((_, tail)) = trimmed.rsplit_once(':') {
        candidates.push(tail.to_string());
    }

    for candidate in &candidates {
        match ctx.db.get_package(candidate) {
            Ok(entry) => return Ok(entry),
            Err(BallError::PackageNotFound(_)) => continue,
            Err(e) => return Err(e),
        }
    }

    Err(BallError::PackageNotFound(trimmed.to_string()))
}

/// What a re-scan of an installed package produced.
enum ScanOutcome {
    /// `--no-scan`, or a source that never produces an artifact
    Skipped,
    /// The recorded install path is no longer on disk
    Missing,
    Scanned(Vec<ScanFinding>),
}

/// Re-scan an installed package's extracted tree, if it is still there.
fn rescan(ctx: &AppContext, pkg: &Package, install_path: &str) -> ScanOutcome {
    if install_path.trim().is_empty() {
        return ScanOutcome::Skipped;
    }

    let path = std::path::Path::new(install_path);
    if !path.is_dir() {
        return ScanOutcome::Missing;
    }

    match ctx.referee.screen_artifact(pkg, path) {
        Ok(findings) => ScanOutcome::Scanned(findings),
        // An audit reports; it does not fail. A blocking finding on an already
        // installed package is the most important thing the command can say,
        // so it is unpacked into the report rather than raised as an error.
        Err(BallError::RefereeScanBlocked { findings, .. }) => ScanOutcome::Scanned(findings),
        Err(e) => {
            tracing::debug!("referee: could not scan {}: {}", pkg.name, e);
            ScanOutcome::Skipped
        }
    }
}

fn report_empty(ctx: &AppContext, package_name: Option<&str>) -> Result<(), BallError> {
    if ctx.flags.json {
        return print_json(&json!({
            "command": "referee",
            "packages": [],
        }));
    }

    match package_name {
        Some(name) => println!("{} '{}' is not on the roster", "Note".yellow(), name),
        None => println!("{} the roster is empty", "Note".yellow()),
    }
    Ok(())
}

fn print_table(ctx: &AppContext, reports: &[PackageReport], scans: &[(String, ScanOutcome)]) {
    let thresholds = ctx.referee.thresholds();

    println!(
        "{:<24} {:<14} {:<12} {:<8} {}",
        "PACKAGE".bold(),
        "VERSION".bold(),
        "STATUS".bold(),
        "RISK".bold(),
        "ADVISORIES".bold()
    );

    for report in reports {
        let status = match report.band(thresholds) {
            Band::Block => report.status().label().red().bold(),
            Band::Warn => report.status().label().yellow().bold(),
            Band::Pass => match report.status() {
                Verdict::Clean => report.status().label().green(),
                _ => report.status().label().normal(),
            },
        };

        let risk = match report.risk() {
            Some(risk) => format!("{:.2}", risk),
            None => "—".to_string(),
        };

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

        println!(
            "{:<24} {:<14} {:<12} {:<8} {}",
            report.name.cyan(),
            report.version.yellow(),
            status,
            risk,
            ids
        );

        for advisory in &advisories {
            println!("    {} {}", "•".yellow(), advisory.describe());
        }

        for identity in report.unchecked() {
            println!(
                "    {} {} ({}) — {}",
                "?".yellow(),
                identity.identity.label(),
                identity.identity.scope.label(),
                identity.status.label()
            );
        }

        if let Some((_, outcome)) = scans.iter().find(|(name, _)| name == &report.name) {
            match outcome {
                ScanOutcome::Skipped => {}
                ScanOutcome::Missing => println!(
                    "    {} artifact not on disk — nothing to re-scan",
                    "?".yellow()
                ),
                ScanOutcome::Scanned(findings) => {
                    for finding in findings {
                        let marker = match finding.severity {
                            ScanSeverity::Block => "!".red().bold(),
                            ScanSeverity::Warn => "!".yellow(),
                        };
                        println!("    {} {}", marker, finding.describe());
                    }
                }
            }
        }
    }

    let blocked = reports
        .iter()
        .filter(|report| report.band(thresholds) == Band::Block)
        .count();
    let warned = reports
        .iter()
        .filter(|report| report.band(thresholds) == Band::Warn)
        .count();
    let unchecked = reports
        .iter()
        .filter(|report| report.status().is_unchecked())
        .count();

    println!();
    println!(
        "{} {} package(s): {} over the block threshold, {} warned, {} not verified",
        "Summary".bold(),
        reports.len(),
        blocked,
        warned,
        unchecked
    );
    println!(
        "{} the audit changes nothing — eject or update a flagged package yourself",
        "Note".yellow()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::package::PackageSource;

    fn row(source: &str, detail: Option<&str>) -> InstalledPackage {
        InstalledPackage {
            name: "tool".to_string(),
            version: "1.2.3".to_string(),
            source: source.to_string(),
            source_detail: detail.map(String::from),
            description: None,
            author: None,
            repository: Some("https://github.com/owner/tool".to_string()),
            download_url: None,
            sha256: None,
            frozen: false,
            user_installed: true,
            install_path: "/cache/tool-1.2.3".to_string(),
            bin_path: None,
            manifest_path: None,
            installed_at: String::new(),
            advisory: None,
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn test_roster_row_becomes_a_checkable_package() {
        let pkg = row("github", Some("owner/tool")).to_package();
        assert_eq!(pkg.name, "tool");
        assert_eq!(pkg.version, "1.2.3");
        assert_eq!(
            pkg.source,
            PackageSource::GitHub {
                owner: "owner".to_string(),
                repo: "tool".to_string()
            }
        );
        assert_eq!(source_label(&pkg.source), "github:owner/tool");
    }

    #[test]
    fn test_system_roster_row_keeps_its_manager() {
        let pkg = row("system", Some("apt")).to_package();
        assert_eq!(
            pkg.source,
            PackageSource::System {
                manager: "apt".to_string()
            }
        );
    }

    #[test]
    fn test_cargo_roster_row_keeps_its_crate_name() {
        let pkg = row("cargo", Some("ripgrep")).to_package();
        assert_eq!(
            pkg.source,
            PackageSource::Cargo {
                crate_name: "ripgrep".to_string()
            }
        );
    }

    #[test]
    fn test_chocolatey_roster_row_keeps_its_feed_and_project_url() {
        let pkg = row("chocolatey", Some("https://feed.test/api/v2")).to_package();
        assert_eq!(
            pkg.source,
            PackageSource::Chocolatey {
                feed_url: "https://feed.test/api/v2".to_string()
            }
        );
        assert_eq!(
            pkg.repository.as_deref(),
            Some("https://github.com/owner/tool")
        );
    }

    #[test]
    fn test_unrecognised_roster_source_yields_no_identity() {
        let pkg = row("quantum-registry", Some("whatever")).to_package();
        assert!(crate::security::identity::advisory_identities(&pkg).is_empty());
    }
}
