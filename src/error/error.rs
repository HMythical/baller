use std::fmt;

use crate::security::scan::ScanFinding;
use crate::security::verdict::MatchedAdvisory;

/// One advisory as a blocked install reports it: id, aliases, score, summary.
pub type AdvisoryRef = MatchedAdvisory;

/// A package the advisory gate refused to install, and why.
#[derive(Debug, Clone)]
pub struct BlockedPackage {
    pub package: String,
    pub version: String,
    pub advisories: Vec<AdvisoryRef>,
    pub reason: String,
}

#[derive(Debug)]
pub enum BallError {
    /// The host cannot run what was asked for: an unsupported OS at startup,
    /// or a package whose source or architectures exclude this host. The
    /// message is complete and displayed verbatim.
    UnsupportedOs(String),
    #[allow(dead_code)]
    UnsupportedCommand(String),
    #[allow(dead_code)]
    UnknownParameter(String),
    FileIoErr(std::io::Error),
    InvalidConfig(String),
    UnknownConfigEntry((usize, String)),
    NetworkError(String),
    PackageNotFound(String),
    HashMismatch(String),
    ExtractionFailed(String),
    DependencyCycle(String),
    VersionConflict(String),
    PackageFrozen(String),
    PackageManagerError(String),
    InjectedCommandError(String),
    PipeRedirected {
        pipe: String,
        msg: String,
    },
    ConfirmationAborted,
    /// A prompt needed an answer under `--json`, where nobody is there to give
    /// one. Carries the question, without its `[y/N]` hint.
    ConfirmationRequired(String),
    /// No release asset matches the host platform.
    ///
    /// A hard error: only `PackageNotFound` is skippable during dependency
    /// resolution (`core::dep_solver`, and only for system virtual packages),
    /// so this aborts the command rather than letting an install proceed with
    /// an asset built for another platform.
    NoMatchingAsset {
        /// The package the release belongs to
        package: String,
        /// The host platform, as `<os>-<arch>`
        platform: String,
        /// Every asset name the release offered, for diagnosis
        assets: Vec<String>,
    },
    /// An archive extracted cleanly but held no executable for this host.
    ///
    /// A hard error for the same reason as `NoMatchingAsset`: the package is
    /// not recorded on the roster, because nothing installable was produced.
    NoBinaryFound {
        /// The package that was extracted
        package: String,
        /// The version that was extracted
        version: String,
        /// The extract directory that was searched
        dir: String,
        /// The cached archive it came from, when one is known
        archive: Option<String>,
    },
    /// An extracted binary is a definite executable for another platform,
    /// e.g. a PE/Windows `.exe` found on a Linux host.
    ///
    /// The backstop behind the source and asset checks: raised from the
    /// artifact's own bytes, whichever source supplied it. A hard error for
    /// the same reason as `NoMatchingAsset`, so nothing is linked or recorded.
    /// Like `NoBinaryFound`, the host (`<os>-<arch>`) is rendered on display,
    /// which keeps `BallError` under clippy's `result_large_err` limit.
    PlatformMismatch {
        /// The package that was extracted
        package: String,
        /// The version that was extracted
        version: String,
        /// What the binary's header says it is, e.g. "PE/Windows executable"
        format: String,
        /// The rejected binary, inside the (removed) extract directory
        binary: String,
        /// The cached archive it came from, which is kept
        archive: Option<String>,
    },
    /// Referee's advisory gate refused the plan.
    ///
    /// Raised before the install loop runs, so nothing has been downloaded,
    /// linked or recorded: the block is all-or-nothing by construction. Every
    /// package that crossed the block threshold is listed, because fixing one
    /// and rediscovering the next one at a time helps nobody.
    RefereeBlocked {
        packages: Vec<BlockedPackage>,
    },
    /// Referee's artifact scan refused a downloaded package.
    ///
    /// Raised after extraction and before linking; the caller purges the
    /// extract directory and the cached archive on the way out. `discarded` is
    /// false only for `build`'s cargo-project path, where the scanned file is
    /// the user's own compiled binary and is deliberately left in place.
    RefereeScanBlocked {
        package: String,
        version: String,
        findings: Vec<ScanFinding>,
        discarded: bool,
    },
    /// The advisory service could not be reached.
    ///
    /// Only fatal under `fail_policy = fail-closed`; the default fail-open
    /// path reports the affected packages as `Unverified` and continues.
    RefereeUnavailable {
        message: String,
    },
    /// `baller referee --fail-on` found packages at or above the chosen level,
    /// through an advisory or an artifact-scan finding.
    ///
    /// Raised after the report is printed, only to turn it into a non-zero
    /// exit for CI. The audit itself is read-only: nothing was changed.
    RefereeAuditFailed {
        /// The `--fail-on` band, `block` or `warn`
        band: &'static str,
        packages: Vec<String>,
    },
    /// Non-optional dependencies that could not be resolved from any
    /// configured source and are not tolerated system virtual packages.
    ///
    /// Raised by the `draft`/`substitute` paths, where installing a subset of
    /// the requested dependency set silently would mask a broken install.
    UnresolvedDependencies(Vec<String>),
}

impl fmt::Display for BallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BallError::UnsupportedOs(msg) => write!(f, "{}", msg),

            BallError::UnsupportedCommand(command) => write!(
                f,
                "the following command does not exist: {}\n\t run 'baller help' for more info",
                command
            ),

            BallError::UnknownParameter(param) => write!(
                f,
                "unknown parameter: '{}'\n\t run 'baller help' for more info about parameters",
                param
            ),

            BallError::FileIoErr(file) => write!(f, "{}", file),

            BallError::InvalidConfig(msg) => write!(f, "{}", msg),

            BallError::UnknownConfigEntry((line, entry)) => {
                write!(f, "unknown config entry at line[{}]: '{}'", line, entry)
            }

            BallError::NetworkError(msg) => write!(f, "network error: {}", msg),

            BallError::PackageNotFound(name) => write!(f, "package not found: {}", name),

            BallError::HashMismatch(msg) => write!(f, "hash verification failed: {}", msg),

            BallError::ExtractionFailed(msg) => write!(f, "failed to extract package: {}", msg),

            BallError::DependencyCycle(msg) => write!(f, "dependency cycle detected: {}", msg),

            BallError::VersionConflict(msg) => write!(f, "version conflict: {}", msg),

            BallError::PackageFrozen(name) => {
                write!(f, "package '{}' is frozen and cannot be modified", name)
            }

            BallError::PackageManagerError(msg) => write!(f, "package manager error: {}", msg),

            BallError::InjectedCommandError(msg) => write!(f, "injected command error: {}", msg),

            BallError::PipeRedirected { pipe, msg } => {
                write!(f, "{}, pipe redirected: {}", msg, pipe)
            }

            BallError::ConfirmationAborted => write!(f, "Aborted"),

            BallError::ConfirmationRequired(question) => write!(
                f,
                "confirmation required: {} — pass --yes/-y to proceed under --json",
                question
            ),

            BallError::NoMatchingAsset {
                package,
                platform,
                assets,
            } => {
                let available = if assets.is_empty() {
                    "none".to_string()
                } else {
                    assets.join(", ")
                };
                write!(
                    f,
                    "no {} asset for '{}' — available: {} (specify a different source or version)",
                    platform, package, available
                )
            }

            BallError::RefereeBlocked { packages } => {
                writeln!(
                    f,
                    "referee blocked {} package(s); nothing was installed",
                    packages.len()
                )?;
                for (index, blocked) in packages.iter().enumerate() {
                    if index > 0 {
                        writeln!(f)?;
                    }
                    write!(
                        f,
                        "\t{} v{} — {}",
                        blocked.package, blocked.version, blocked.reason
                    )?;
                    for advisory in &blocked.advisories {
                        write!(f, "\n\t  • {}", advisory.describe())?;
                    }
                }
                write!(
                    f,
                    "\n\trun with --no-referee to install anyway, or raise referee.block_at"
                )
            }

            BallError::RefereeScanBlocked {
                package,
                version,
                findings,
                discarded,
            } => {
                if *discarded {
                    writeln!(
                        f,
                        "referee blocked the downloaded archive for '{}' v{} — the download was discarded",
                        package, version
                    )?;
                } else {
                    writeln!(
                        f,
                        "referee blocked the compiled binary for '{}' v{} — it was left in place and nothing was linked",
                        package, version
                    )?;
                }
                // Each bullet leads with its severity in capitals, so the
                // finding that caused the block stands out from the warnings
                // listed beside it. Built from the fields rather than
                // `describe()`, which already starts with the lowercase label
                // and is shared with the Markdown, JSON and SARIF writers.
                for (index, finding) in findings.iter().enumerate() {
                    if index > 0 {
                        writeln!(f)?;
                    }
                    write!(
                        f,
                        "\t• {}: [{}] {} — {}",
                        finding.severity.label().to_uppercase(),
                        finding.rule.label(),
                        finding.path.display(),
                        finding.evidence
                    )?;
                }
                Ok(())
            }

            BallError::RefereeUnavailable { message } => {
                write!(f, "referee could not verify this install: {}", message)
            }

            BallError::RefereeAuditFailed { band, packages } => write!(
                f,
                "referee found {} package(s) at or above the '{}' level (--fail-on {}): {}",
                packages.len(),
                band,
                band,
                packages.join(", ")
            ),

            BallError::NoBinaryFound {
                package,
                version,
                dir,
                archive,
            } => write!(
                f,
                "no binary found in extracted package '{}' v{} — expected an executable for {} in {} (archive: {})",
                package,
                version,
                std::env::consts::OS,
                dir,
                archive.as_deref().unwrap_or("<none>")
            ),

            BallError::PlatformMismatch {
                package,
                version,
                format,
                binary,
                archive,
            } => write!(
                f,
                "package '{}' v{} ships a binary this {}-{} host cannot run ({}) — rejected {} (archive: {})",
                package,
                version,
                std::env::consts::OS,
                std::env::consts::ARCH,
                format,
                binary,
                archive.as_deref().unwrap_or("<none>")
            ),

            BallError::UnresolvedDependencies(names) => write!(
                f,
                "unresolved dependencies: {} could not be found in any configured registry",
                names.join(", ")
            ),
        }
    }
}

impl BallError {
    /// The variant name, as the `code` field of a `--json` error document.
    ///
    /// Stable across message rewording, so a script can branch on it without
    /// parsing the human text.
    pub fn code(&self) -> &'static str {
        match self {
            BallError::UnsupportedOs(_) => "UnsupportedOs",
            BallError::UnsupportedCommand(_) => "UnsupportedCommand",
            BallError::UnknownParameter(_) => "UnknownParameter",
            BallError::FileIoErr(_) => "FileIoErr",
            BallError::InvalidConfig(_) => "InvalidConfig",
            BallError::UnknownConfigEntry(_) => "UnknownConfigEntry",
            BallError::NetworkError(_) => "NetworkError",
            BallError::PackageNotFound(_) => "PackageNotFound",
            BallError::HashMismatch(_) => "HashMismatch",
            BallError::ExtractionFailed(_) => "ExtractionFailed",
            BallError::DependencyCycle(_) => "DependencyCycle",
            BallError::VersionConflict(_) => "VersionConflict",
            BallError::PackageFrozen(_) => "PackageFrozen",
            BallError::PackageManagerError(_) => "PackageManagerError",
            BallError::InjectedCommandError(_) => "InjectedCommandError",
            BallError::PipeRedirected { .. } => "PipeRedirected",
            BallError::ConfirmationAborted => "ConfirmationAborted",
            BallError::ConfirmationRequired(_) => "ConfirmationRequired",
            BallError::NoMatchingAsset { .. } => "NoMatchingAsset",
            BallError::NoBinaryFound { .. } => "NoBinaryFound",
            BallError::PlatformMismatch { .. } => "PlatformMismatch",
            BallError::RefereeBlocked { .. } => "RefereeBlocked",
            BallError::RefereeScanBlocked { .. } => "RefereeScanBlocked",
            BallError::RefereeUnavailable { .. } => "RefereeUnavailable",
            BallError::RefereeAuditFailed { .. } => "RefereeAuditFailed",
            BallError::UnresolvedDependencies(_) => "UnresolvedDependencies",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsupported_os_display() {
        // Displayed verbatim: the variant also carries platform-gate messages
        // that must not be wrapped in the startup "unsupported OS" sentence.
        let msg = "'vim' uses the system source, which only serves linux hosts";
        let err = BallError::UnsupportedOs(msg.to_string());
        assert_eq!(format!("{}", err), msg);
    }

    #[test]
    fn test_unsupported_command_display() {
        let err = BallError::UnsupportedCommand("foobar".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("foobar"));
        assert!(msg.contains("command does not exist"));
    }

    #[test]
    fn test_unknown_parameter_display() {
        let err = BallError::UnknownParameter("--xyz".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("--xyz"));
    }

    #[test]
    fn test_file_io_err_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = BallError::FileIoErr(io_err);
        let msg = format!("{}", err);
        assert!(msg.contains("file not found"));
    }

    #[test]
    fn test_invalid_config_display() {
        let err = BallError::InvalidConfig("bad config".to_string());
        assert_eq!(format!("{}", err), "bad config");
    }

    #[test]
    fn test_unknown_config_entry_display() {
        let err = BallError::UnknownConfigEntry((5, "foo".to_string()));
        let msg = format!("{}", err);
        assert!(msg.contains("line[5]"));
        assert!(msg.contains("foo"));
    }

    #[test]
    fn test_network_error_display() {
        let err = BallError::NetworkError("timeout".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("network error"));
        assert!(msg.contains("timeout"));
    }

    #[test]
    fn test_package_not_found_display() {
        let err = BallError::PackageNotFound("myapp".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("package not found"));
        assert!(msg.contains("myapp"));
    }

    #[test]
    fn test_hash_mismatch_display() {
        let err = BallError::HashMismatch("expected abc got def".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("hash verification failed"));
        assert!(msg.contains("expected abc got def"));
    }

    #[test]
    fn test_extraction_failed_display() {
        let err = BallError::ExtractionFailed("corrupt archive".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("failed to extract package"));
        assert!(msg.contains("corrupt archive"));
    }

    #[test]
    fn test_dependency_cycle_display() {
        let err = BallError::DependencyCycle("a -> b -> a".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("dependency cycle detected"));
        assert!(msg.contains("a -> b -> a"));
    }

    #[test]
    fn test_version_conflict_display() {
        let err = BallError::VersionConflict("a v1 vs b v2".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("version conflict"));
        assert!(msg.contains("a v1 vs b v2"));
    }

    #[test]
    fn test_package_frozen_display() {
        let err = BallError::PackageFrozen("myapp".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("myapp"));
        assert!(msg.contains("frozen"));
    }

    #[test]
    fn test_injected_command_error_display() {
        let err = BallError::InjectedCommandError("'my-tool' is not injected".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("injected command error"));
        assert!(msg.contains("'my-tool' is not injected"));
    }

    #[test]
    fn test_no_matching_asset_display() {
        let err = BallError::NoMatchingAsset {
            package: "ripgrep".to_string(),
            platform: "linux-x86_64".to_string(),
            assets: vec![
                "ripgrep-aarch64-apple-darwin.tar.gz".to_string(),
                "ripgrep.deb".to_string(),
            ],
        };
        let msg = format!("{}", err);
        assert!(msg.contains("no linux-x86_64 asset for 'ripgrep'"));
        assert!(msg.contains("ripgrep-aarch64-apple-darwin.tar.gz, ripgrep.deb"));
        assert!(msg.contains("specify a different source or version"));
    }

    #[test]
    fn test_no_matching_asset_display_with_no_assets() {
        let err = BallError::NoMatchingAsset {
            package: "empty".to_string(),
            platform: "windows-x86_64".to_string(),
            assets: Vec::new(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("available: none"));
    }

    #[test]
    fn test_no_binary_found_display() {
        let err = BallError::NoBinaryFound {
            package: "tool".to_string(),
            version: "1.2.3".to_string(),
            dir: "/cache/tool-1.2.3".to_string(),
            archive: Some("/cache/tool.tar.gz".to_string()),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("no binary found in extracted package 'tool' v1.2.3"));
        assert!(msg.contains("/cache/tool-1.2.3"));
        assert!(msg.contains("archive: /cache/tool.tar.gz"));
    }

    #[test]
    fn test_no_binary_found_display_without_archive() {
        let err = BallError::NoBinaryFound {
            package: "tool".to_string(),
            version: "1.2.3".to_string(),
            dir: "/cache/tool-1.2.3".to_string(),
            archive: None,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("archive: <none>"));
    }

    #[test]
    fn test_platform_mismatch_display() {
        let err = BallError::PlatformMismatch {
            package: "7zip".to_string(),
            version: "24.8.0".to_string(),
            format: "PE/Windows executable".to_string(),
            binary: "/cache/7zip-24.8.0/7zip.exe".to_string(),
            archive: Some("/cache/7zip.nupkg".to_string()),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("package '7zip' v24.8.0"));
        let host = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        assert!(msg.contains(&format!("ships a binary this {} host cannot run", host)));
        assert!(msg.contains("(PE/Windows executable)"));
        assert!(msg.contains("rejected /cache/7zip-24.8.0/7zip.exe"));
        assert!(msg.contains("archive: /cache/7zip.nupkg"));
    }

    #[test]
    fn test_platform_mismatch_display_without_archive() {
        let err = BallError::PlatformMismatch {
            package: "tool".to_string(),
            version: "1.0.0".to_string(),
            format: "ELF/Linux executable".to_string(),
            binary: "C:\\cache\\tool-1.0.0\\tool".to_string(),
            archive: None,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("host cannot run (ELF/Linux executable)"));
        assert!(msg.contains("archive: <none>"));
    }

    #[test]
    fn test_unresolved_dependencies_display() {
        let err = BallError::UnresolvedDependencies(vec!["foo".to_string(), "bar".to_string()]);
        let msg = format!("{}", err);
        assert!(msg.contains("unresolved dependencies"));
        assert!(msg.contains("foo, bar"));
        assert!(msg.contains("in any configured registry"));
    }

    #[test]
    fn test_referee_audit_failed_display() {
        let err = BallError::RefereeAuditFailed {
            band: "warn",
            packages: vec!["alpha".to_string(), "beta".to_string()],
        };
        let msg = format!("{}", err);
        assert!(msg.contains("2 package(s)"));
        assert!(msg.contains("--fail-on warn"));
        assert!(msg.contains("alpha, beta"));
    }

    #[test]
    fn test_debug_format() {
        let err = BallError::PackageNotFound("test".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("PackageNotFound"));
    }

    #[test]
    fn test_confirmation_required_display() {
        let err = BallError::ConfirmationRequired("Are you sure you want to eject fd?".to_string());
        let msg = format!("{}", err);
        assert!(msg.starts_with("confirmation required: Are you sure you want to eject fd?"));
        assert!(msg.contains("--yes"));
    }

    #[test]
    fn test_code_is_the_variant_name() {
        // The Debug form starts with the variant name, so the two must agree.
        let errors = [
            BallError::PackageNotFound("x".to_string()),
            BallError::FileIoErr(std::io::Error::other("x")),
            BallError::ConfirmationAborted,
            BallError::ConfirmationRequired("x".to_string()),
            BallError::PipeRedirected {
                pipe: "stdin".to_string(),
                msg: "x".to_string(),
            },
            BallError::RefereeAuditFailed {
                band: "warn",
                packages: vec![],
            },
            BallError::UnresolvedDependencies(vec![]),
        ];
        for err in &errors {
            assert!(
                format!("{:?}", err).starts_with(err.code()),
                "code '{}' does not match {:?}",
                err.code(),
                err
            );
        }
    }

    #[test]
    fn test_scan_blocked_bullets_carry_a_severity_label() {
        use crate::security::scan::{ScanRule, ScanSeverity};
        use std::path::PathBuf;

        let finding = |severity, rule| ScanFinding {
            path: PathBuf::from("bin/tool"),
            rule,
            severity,
            evidence: "evidence".to_string(),
        };
        let err = BallError::RefereeScanBlocked {
            package: "tool".to_string(),
            version: "1.0.0".to_string(),
            findings: vec![
                finding(ScanSeverity::Block, ScanRule::VirusTotalDetection),
                finding(ScanSeverity::Warn, ScanRule::HighEntropy),
            ],
            discarded: true,
        };
        let text = err.to_string();
        assert!(text.starts_with(
            "referee blocked the downloaded archive for 'tool' v1.0.0 — the download was discarded\n"
        ));
        assert!(text.contains("\t• BLOCK: [virustotal-detection] bin/tool — evidence"));
        assert!(text.contains("\t• WARN: [high-entropy] bin/tool — evidence"));
    }

    #[test]
    fn test_scan_blocked_on_a_kept_binary_does_not_claim_a_discard() {
        let err = BallError::RefereeScanBlocked {
            package: "tool".to_string(),
            version: "1.0.0".to_string(),
            findings: Vec::new(),
            discarded: false,
        };
        let text = err.to_string();
        assert!(!text.contains("discarded"));
        assert!(text.contains("compiled binary for 'tool' v1.0.0 — it was left in place"));
    }
}
