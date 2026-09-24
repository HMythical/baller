//! `baller referee` — audit the roster against public vulnerability data.
//!
//! The audit is deliberately **read-only**. A package that would be blocked on
//! install is still on the roster because it was installed before the advisory
//! existed, or before Referee did; silently ejecting it would be a far worse
//! surprise than reporting it. The command's job is to tell the user what they
//! are running, and let them decide.
//!
//! The command is a group: `audit` (the default, both phases), `check`
//! (Phase A only), `scan` (Phase B only), `cache`, `config` and `sbom`. Every
//! subcommand is read-only with respect to the roster; `cache` is the only one
//! that writes, and only to the verdict cache.

mod audit;
mod cache;
mod check;
mod config;
mod export;
mod sbom;
mod scan;

use clap::ValueEnum;
use colored::Colorize;
use serde_json::json;

#[cfg(test)]
use crate::commands::draft::source_label;
use crate::context::AppContext;
use crate::core::db::InstalledPackage;
use crate::core::package::Package;
use crate::error::error::BallError;
use crate::security::export::{scan_for, ScanOutcome};
use crate::security::scan::ScanSeverity;
use crate::security::scoring::Band;
use crate::security::verdict::{PackageReport, Verdict};
use crate::security::GateOutcome;
use crate::utils::output::print_json;

pub use audit::AuditOptions;
pub use cache::CacheAction;

/// The band `--fail-on` turns into a non-zero exit.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum FailOn {
    /// Fail when any package crosses `block_at`
    Block,
    /// Fail when any package crosses `warn_at` (blocked packages included)
    Warn,
}

impl FailOn {
    pub fn label(self) -> &'static str {
        match self {
            FailOn::Block => "block",
            FailOn::Warn => "warn",
        }
    }
}

/// What `audit --format` renders.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum AuditFormat {
    /// The `--json` document
    Json,
    /// The report table as a Markdown document
    Markdown,
    /// A SARIF 2.1.0 log, for code-scanning tools
    Sarif,
}

/// What `sbom --format` renders.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum SbomFormat {
    /// CycloneDX 1.5 JSON
    CyclonedxJson,
}

/// A parsed `baller referee` invocation.
pub enum RefereeCommand {
    Audit(AuditOptions),
    Check {
        package_names: Vec<String>,
        refresh: bool,
        fail_on: Option<FailOn>,
    },
    Scan {
        package_names: Vec<String>,
    },
    Cache(CacheAction),
    Config,
    Sbom {
        out: Option<String>,
        format: SbomFormat,
    },
}

pub fn execute_referee(ctx: &AppContext, command: &RefereeCommand) -> Result<(), BallError> {
    match command {
        RefereeCommand::Audit(opts) => audit::execute_audit(ctx, opts),
        RefereeCommand::Check {
            package_names,
            refresh,
            fail_on,
        } => check::execute_check(ctx, package_names, *refresh, *fail_on),
        RefereeCommand::Scan { package_names } => scan::execute_scan(ctx, package_names),
        RefereeCommand::Cache(action) => cache::execute_cache(ctx, *action),
        RefereeCommand::Config => config::execute_config(ctx),
        RefereeCommand::Sbom { out, format } => sbom::execute_sbom(ctx, out.as_deref(), *format),
    }
}

/// The subcommands that consult Referee refuse to run with it switched off.
fn require_enabled(ctx: &AppContext) -> Result<(), BallError> {
    if ctx.referee.enabled() {
        return Ok(());
    }
    Err(BallError::InvalidConfig(
        "referee is disabled — remove --no-referee, or set 'enabled = true' under [referee] in baller.conf"
            .to_string(),
    ))
}

/// The roster rows a command acts on: the named packages, or the whole roster
/// when none were named. A package named twice is audited once.
fn select_roster(
    ctx: &AppContext,
    package_names: &[String],
) -> Result<Vec<InstalledPackage>, BallError> {
    if package_names.is_empty() {
        return ctx.db.list_packages();
    }

    let mut selected: Vec<InstalledPackage> = Vec::with_capacity(package_names.len());
    for name in package_names {
        let entry = resolve_roster_entry(ctx, name)?;
        if !selected.iter().any(|row| row.name == entry.name) {
            selected.push(entry);
        }
    }
    Ok(selected)
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

/// The error `--fail-on` turns an audit into, when anything reached its band.
///
/// Uses the gate's own banding, so `--fail-on block` fails on exactly the
/// packages an install would refuse, and `--fail-on warn` adds the warned ones.
pub(crate) fn fail_on_error(outcome: &GateOutcome, fail_on: Option<FailOn>) -> Option<BallError> {
    let fail_on = fail_on?;
    let mut failing = outcome.blocked();
    if fail_on == FailOn::Warn {
        failing.extend(outcome.warned());
    }
    if failing.is_empty() {
        return None;
    }

    Some(BallError::RefereeAuditFailed {
        band: fail_on.label(),
        packages: failing
            .iter()
            .map(|report| format!("{} v{}", report.name, report.version))
            .collect(),
    })
}

fn report_empty(ctx: &AppContext) -> Result<(), BallError> {
    if ctx.flags.json {
        return print_json(&json!({
            "command": "referee",
            "packages": [],
        }));
    }

    println!("{} the roster is empty", "Note".yellow());
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

        if let Some(outcome) = scan_for(scans, &report.name) {
            print_scan_lines(outcome);
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

/// The indented lines a re-scan adds under a package's row.
fn print_scan_lines(outcome: &ScanOutcome) {
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
