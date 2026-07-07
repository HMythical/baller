use std::fmt;

#[derive(Debug)]
pub enum BallError {
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
}

impl fmt::Display for BallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BallError::UnsupportedOs(os) => write!(
                f,
                "the following OS is unsupported: {}\n\t please use Windows or Linux",
                os
            ),

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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsupported_os_display() {
        let err = BallError::UnsupportedOs("windows".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("windows"));
        assert!(msg.contains("unsupported"));
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
    fn test_debug_format() {
        let err = BallError::PackageNotFound("test".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("PackageNotFound"));
    }
}
