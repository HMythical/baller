use std::fmt;

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
    fn test_debug_format() {
        let err = BallError::PackageNotFound("test".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("PackageNotFound"));
    }
}
