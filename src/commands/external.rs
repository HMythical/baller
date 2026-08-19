use std::path::Path;
use std::process::{Command, Stdio};

use colored::Colorize;

use crate::context::AppContext;
use crate::core::injected::{find_command, resolve_baller_dir, InjectedCommand};
use crate::error::error::BallError;

/// Runs a previously injected command, forwarding `args` to its binary.
///
/// Called for any subcommand clap does not recognize, so an unknown name here
/// is simply an unknown baller command.
pub fn execute_external(ctx: &AppContext, name: &str, args: &[String]) -> Result<(), BallError> {
    let baller_dir = resolve_baller_dir(&ctx.config);

    let command = find_command(&baller_dir, name)
        .ok_or_else(|| BallError::UnsupportedCommand(name.to_string()))?;

    if command.require_root {
        check_root(&command)?;
    }

    check_dependencies(&command)?;

    if !command.path.exists() {
        return Err(BallError::InjectedCommandError(format!(
            "binary for '{}' is missing from '{}': re-inject it or fix the path",
            command.command_name,
            command.path.display()
        )));
    }

    let status = Command::new(&command.path)
        .args(args)
        .status()
        .map_err(|e| {
            BallError::InjectedCommandError(format!(
                "failed to run '{}' ({}): {}",
                command.command_name,
                command.path.display(),
                e
            ))
        })?;

    match status.code() {
        Some(0) | None => Ok(()),
        // Exit with the child's own status so scripts wrapping baller see it.
        Some(code) => std::process::exit(code),
    }
}

/// Refuses to run a root-only command unprivileged; warns when elevation
/// cannot be determined rather than blocking the user.
fn check_root(command: &InjectedCommand) -> Result<(), BallError> {
    match is_elevated() {
        Some(true) => Ok(()),
        Some(false) => Err(BallError::InjectedCommandError(format!(
            "'{}' requires root privileges: re-run baller with elevated permissions",
            command.command_name
        ))),
        None => {
            eprintln!(
                "{} could not verify elevated privileges for '{}'; running anyway",
                "Warning:".yellow().bold(),
                command.command_name
            );
            Ok(())
        }
    }
}

/// Verifies every declared dependency resolves before spawning the binary.
fn check_dependencies(command: &InjectedCommand) -> Result<(), BallError> {
    for dependency in &command.depends {
        if !dependency_available(dependency) {
            return Err(BallError::InjectedCommandError(format!(
                "'{}' requires '{}', which was not found on PATH",
                command.command_name, dependency
            )));
        }
    }
    Ok(())
}

/// True when `dependency` is an existing path or resolves on PATH.
fn dependency_available(dependency: &str) -> bool {
    if Path::new(dependency).exists() {
        return true;
    }

    let probe = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };

    Command::new(probe)
        .arg(dependency)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// `Some(true)`/`Some(false)` when privileges could be determined, `None` when
/// the probe itself failed.
#[cfg(not(target_os = "windows"))]
fn is_elevated() -> Option<bool> {
    let output = Command::new("id").arg("-u").output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim() == "0")
}

#[cfg(target_os = "windows")]
fn is_elevated() -> Option<bool> {
    // S-1-16-12288 is the high mandatory integrity level of an elevated shell.
    let output = Command::new("whoami").arg("/groups").output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).contains("S-1-16-12288"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample(name: &str) -> InjectedCommand {
        InjectedCommand {
            command_name: name.to_string(),
            description: String::new(),
            version: "1.0.0".to_string(),
            flags: vec![],
            author: String::new(),
            require_root: false,
            depends: vec![],
            path: PathBuf::from("/usr/local/bin/my-tool"),
        }
    }

    #[test]
    fn test_dependency_available_for_existing_path() {
        let existing = if cfg!(target_os = "windows") {
            "C:\\Windows"
        } else {
            "/bin"
        };
        assert!(dependency_available(existing));
    }

    #[test]
    fn test_dependency_available_for_path_lookup() {
        let common = if cfg!(target_os = "windows") {
            "cmd"
        } else {
            "sh"
        };
        assert!(dependency_available(common));
    }

    #[test]
    fn test_dependency_missing() {
        assert!(!dependency_available("definitely-not-a-real-binary-xyz123"));
    }

    #[test]
    fn test_check_dependencies_passes_when_all_present() {
        let mut command = sample("my-tool");
        command.depends = vec![if cfg!(target_os = "windows") {
            "cmd".to_string()
        } else {
            "sh".to_string()
        }];
        assert!(check_dependencies(&command).is_ok());
    }

    #[test]
    fn test_check_dependencies_reports_missing_dependency() {
        let mut command = sample("my-tool");
        command.depends = vec!["definitely-not-a-real-binary-xyz123".to_string()];

        let err = check_dependencies(&command).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("my-tool"));
        assert!(msg.contains("definitely-not-a-real-binary-xyz123"));
        assert!(msg.contains("not found on PATH"));
    }

    #[test]
    fn test_check_dependencies_noop_without_dependencies() {
        assert!(check_dependencies(&sample("my-tool")).is_ok());
    }

    #[test]
    fn test_is_elevated_reports_a_decision() {
        // The probe is expected to succeed on supported platforms; the value
        // itself depends on how the test suite was launched.
        assert!(is_elevated().is_some());
    }

    #[test]
    fn test_check_root_errors_when_unprivileged() {
        if is_elevated() == Some(false) {
            let mut command = sample("my-tool");
            command.require_root = true;
            let err = check_root(&command).unwrap_err();
            assert!(format!("{}", err).contains("requires root privileges"));
        }
    }
}
