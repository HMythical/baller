use std::path::Path;
use std::process::Command;

use crate::config::config::HooksConfig;
use crate::error::error::BallError;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HookType {
    PreInstall,
    PostInstall,
    PreEject,
    PostEject,
    PreUpdate,
    PostUpdate,
}

impl HookType {
    pub(crate) fn type_str(&self) -> &'static str {
        match self {
            HookType::PreInstall => "pre_install",
            HookType::PostInstall => "post_install",
            HookType::PreEject => "pre_eject",
            HookType::PostEject => "post_eject",
            HookType::PreUpdate => "pre_update",
            HookType::PostUpdate => "post_update",
        }
    }

    fn is_enabled(&self, config: &HooksConfig) -> bool {
        match self {
            HookType::PreInstall => config.pre_install,
            HookType::PostInstall => config.post_install,
            HookType::PreEject => config.pre_eject,
            HookType::PostEject => config.post_eject,
            HookType::PreUpdate => config.pre_update,
            HookType::PostUpdate => config.post_update,
        }
    }

    /// Whether a failing hook of this type cancels the operation.
    ///
    /// Pre-hooks run before anything changes, so they can veto it. Post-hooks
    /// run once the install, eject or update has already been applied: failing
    /// the command then would report work that happened as an error, so their
    /// failures are logged instead.
    fn aborts_on_failure(&self) -> bool {
        matches!(
            self,
            HookType::PreInstall | HookType::PreEject | HookType::PreUpdate
        )
    }

    /// The operation a hook of this type runs around, for messages
    fn operation(&self) -> &'static str {
        match self {
            HookType::PreInstall | HookType::PostInstall => "install",
            HookType::PreEject | HookType::PostEject => "eject",
            HookType::PreUpdate | HookType::PostUpdate => "update",
        }
    }
}

fn script_filename(hook_type: &HookType, pkg_name: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        format!("{}_{}.ps1", pkg_name, hook_type.type_str())
    }
    #[cfg(target_os = "linux")]
    {
        format!("{}_{}.sh", pkg_name, hook_type.type_str())
    }
}

/// The interpreter invocation for a hook script.
///
/// On Windows the script runs with `-ExecutionPolicy Bypass`: a stock Windows
/// client has an effective policy of `Restricted`, which refuses every script
/// file, so without it no hook could ever run. The user placed the script in
/// their own hooks directory, which is the consent `bash <script>` relies on
/// on Linux. `-NoProfile` likewise matches non-interactive bash, which reads
/// no rc file. A policy enforced by Group Policy still overrides the flag, and
/// PowerShell reports that refusal itself.
fn hook_command(script_path: &Path) -> Command {
    #[cfg(target_os = "linux")]
    let cmd = {
        let mut c = Command::new("bash");
        c.arg(script_path);
        c
    };

    #[cfg(target_os = "windows")]
    let cmd = {
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script_path);
        c
    };

    cmd
}

/// Run the user's hook script for `hook_type`, if one exists and is enabled.
///
/// A failing pre-hook is an error, which cancels the operation. A failing
/// post-hook is logged as a warning and `Ok` is returned: the operation it
/// follows has already been applied, so the command still succeeds.
pub fn run_hook(
    hook_type: &HookType,
    pkg_name: &str,
    pkg_version: &str,
    hooks_dir: &Path,
    config: &HooksConfig,
    extra_env: &[(&str, &str)],
) -> Result<(), BallError> {
    if !hook_type.is_enabled(config) {
        return Ok(());
    }

    if !hooks_dir.exists() {
        return Ok(());
    }

    let script_name = script_filename(hook_type, pkg_name);
    let script_path = hooks_dir.join(&script_name);

    if !script_path.exists() {
        return Ok(());
    }

    match execute_hook(
        hook_type,
        &script_name,
        &script_path,
        pkg_name,
        pkg_version,
        extra_env,
    ) {
        Err(err) if !hook_type.aborts_on_failure() => {
            tracing::warn!(
                "{} — a failing {} hook does not undo the {} of '{}', which completed",
                err,
                hook_type.type_str(),
                hook_type.operation(),
                pkg_name
            );
            Ok(())
        }
        result => result,
    }
}

fn execute_hook(
    hook_type: &HookType,
    script_name: &str,
    script_path: &Path,
    pkg_name: &str,
    pkg_version: &str,
    extra_env: &[(&str, &str)],
) -> Result<(), BallError> {
    let mut cmd = hook_command(script_path);

    cmd.env("BALLER_PACKAGE_NAME", pkg_name);
    cmd.env("BALLER_PACKAGE_VERSION", pkg_version);
    cmd.env("BALLER_HOOK_TYPE", hook_type.type_str());

    for (k, v) in extra_env {
        cmd.env(k, v);
    }

    let status = cmd.status().map_err(|e| {
        BallError::InvalidConfig(format!("failed to execute hook '{}': {}", script_name, e))
    })?;

    if !status.success() {
        let outcome = match status.code() {
            Some(code) => format!("exit code {}", code),
            None => "no exit code (terminated by a signal)".to_string(),
        };
        return Err(BallError::InvalidConfig(format!(
            "hook '{}' failed with {}",
            script_name, outcome
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hooks_config() -> HooksConfig {
        HooksConfig {
            pre_install: true,
            post_install: true,
            pre_eject: true,
            post_eject: true,
            pre_update: true,
            post_update: true,
        }
    }

    #[test]
    fn test_hook_type_type_str() {
        assert_eq!(HookType::PreInstall.type_str(), "pre_install");
        assert_eq!(HookType::PostInstall.type_str(), "post_install");
        assert_eq!(HookType::PreEject.type_str(), "pre_eject");
        assert_eq!(HookType::PostEject.type_str(), "post_eject");
        assert_eq!(HookType::PreUpdate.type_str(), "pre_update");
        assert_eq!(HookType::PostUpdate.type_str(), "post_update");
    }

    #[test]
    fn test_hook_type_is_enabled_all_true() {
        let config = test_hooks_config();
        assert!(HookType::PreInstall.is_enabled(&config));
        assert!(HookType::PostInstall.is_enabled(&config));
        assert!(HookType::PreEject.is_enabled(&config));
        assert!(HookType::PostEject.is_enabled(&config));
        assert!(HookType::PreUpdate.is_enabled(&config));
        assert!(HookType::PostUpdate.is_enabled(&config));
    }

    #[test]
    fn test_hook_type_is_enabled_all_false() {
        let config = HooksConfig {
            pre_install: false,
            post_install: false,
            pre_eject: false,
            post_eject: false,
            pre_update: false,
            post_update: false,
        };
        assert!(!HookType::PreInstall.is_enabled(&config));
        assert!(!HookType::PostInstall.is_enabled(&config));
    }

    #[test]
    fn test_hook_type_is_enabled_selective() {
        let config = HooksConfig {
            pre_install: true,
            post_install: false,
            pre_eject: true,
            post_eject: false,
            pre_update: true,
            post_update: false,
        };
        assert!(HookType::PreInstall.is_enabled(&config));
        assert!(!HookType::PostInstall.is_enabled(&config));
        assert!(HookType::PreEject.is_enabled(&config));
        assert!(!HookType::PostEject.is_enabled(&config));
        assert!(HookType::PreUpdate.is_enabled(&config));
        assert!(!HookType::PostUpdate.is_enabled(&config));
    }

    #[test]
    fn test_hook_type_eq() {
        assert_eq!(HookType::PreInstall, HookType::PreInstall);
        assert_ne!(HookType::PreInstall, HookType::PostInstall);
    }

    #[test]
    fn test_hook_type_copy() {
        let a = HookType::PreEject;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn test_script_filename_linux_format() {
        let name = script_filename(&HookType::PreInstall, "mypkg");
        assert!(name.contains("mypkg"));
        assert!(name.contains("pre_install"));
        #[cfg(target_os = "linux")]
        assert!(name.ends_with(".sh"));
        #[cfg(target_os = "windows")]
        assert!(name.ends_with(".ps1"));
    }

    #[test]
    fn test_run_hook_disabled() {
        let config = HooksConfig {
            pre_install: false,
            post_install: false,
            pre_eject: false,
            post_eject: false,
            pre_update: false,
            post_update: false,
        };
        let dir = std::env::temp_dir().join("baller_test_hooks_disabled");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let result = run_hook(&HookType::PreInstall, "test", "1.0", &dir, &config, &[]);
        assert!(result.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_run_hook_no_hooks_dir() {
        let config = test_hooks_config();
        let dir = Path::new("/nonexistent_hooks_dir_xyz");
        let result = run_hook(&HookType::PreInstall, "test", "1.0", dir, &config, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_hook_extra_env() {
        let config = test_hooks_config();
        let dir = std::env::temp_dir().join("baller_test_hooks_extra_env");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let result = run_hook(
            &HookType::PostInstall,
            "testpkg",
            "2.0.0",
            &dir,
            &config,
            &[("CUSTOM_VAR", "custom_value")],
        );
        // No script exists, so it should return Ok
        assert!(result.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A hooks directory no other test shares
    fn unique_hooks_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "baller_test_hooks_{}_{}_{}",
            tag,
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write the platform's hook script for `hook_type` that touches
    /// `marker` and exits with `code`
    fn write_hook(dir: &Path, hook_type: &HookType, marker: &Path, code: i32) {
        let script = dir.join(script_filename(hook_type, "pkg"));
        #[cfg(target_os = "linux")]
        let body = format!("touch '{}'\nexit {}\n", marker.display(), code);
        #[cfg(target_os = "windows")]
        let body = format!(
            "New-Item -ItemType File -Force '{}' | Out-Null\r\nexit {}\r\n",
            marker.display(),
            code
        );
        std::fs::write(script, body).unwrap();
    }

    const ALL_TYPES: [HookType; 6] = [
        HookType::PreInstall,
        HookType::PostInstall,
        HookType::PreEject,
        HookType::PostEject,
        HookType::PreUpdate,
        HookType::PostUpdate,
    ];

    #[test]
    fn test_only_pre_hooks_abort_on_failure() {
        for hook_type in ALL_TYPES {
            let is_pre = hook_type.type_str().starts_with("pre_");
            assert_eq!(hook_type.aborts_on_failure(), is_pre, "{:?}", hook_type);
        }
    }

    #[test]
    fn test_successful_hook_runs() {
        // On Windows this runs under the host's own execution policy: on a
        // stock client (`Restricted`) it only passes because of the Bypass flag
        let dir = unique_hooks_dir("ok");
        let marker = dir.join("ran");
        write_hook(&dir, &HookType::PreInstall, &marker, 0);

        let result = run_hook(
            &HookType::PreInstall,
            "pkg",
            "1.0.0",
            &dir,
            &test_hooks_config(),
            &[],
        );
        assert!(result.is_ok(), "{:?}", result);
        assert!(marker.exists(), "the hook script did not run");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_failing_pre_hook_aborts_with_its_exit_code() {
        for hook_type in [
            HookType::PreInstall,
            HookType::PreEject,
            HookType::PreUpdate,
        ] {
            let dir = unique_hooks_dir("pre_fail");
            let marker = dir.join("ran");
            write_hook(&dir, &hook_type, &marker, 3);

            match run_hook(&hook_type, "pkg", "1.0.0", &dir, &test_hooks_config(), &[]) {
                Err(BallError::InvalidConfig(msg)) => {
                    assert!(msg.contains("failed with exit code 3"), "{}", msg);
                    assert!(!msg.contains("Some("), "{}", msg);
                }
                other => panic!("{:?}: expected InvalidConfig, got {:?}", hook_type, other),
            }
            assert!(marker.exists(), "{:?} did not run", hook_type);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn test_failing_post_hook_is_logged_not_fatal() {
        for hook_type in [
            HookType::PostInstall,
            HookType::PostEject,
            HookType::PostUpdate,
        ] {
            let dir = unique_hooks_dir("post_fail");
            let marker = dir.join("ran");
            write_hook(&dir, &hook_type, &marker, 3);

            let result = run_hook(&hook_type, "pkg", "1.0.0", &dir, &test_hooks_config(), &[]);
            assert!(result.is_ok(), "{:?}: {:?}", hook_type, result);
            assert!(marker.exists(), "{:?} did not run", hook_type);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn test_hook_command_interpreter_and_flags() {
        let script = Path::new("hooks").join("pkg_pre_install.ext");
        let cmd = hook_command(&script);
        let args: Vec<_> = cmd.get_args().collect();

        #[cfg(target_os = "linux")]
        {
            assert_eq!(cmd.get_program(), "bash");
            assert_eq!(args, vec![script.as_os_str()]);
        }

        #[cfg(target_os = "windows")]
        {
            assert_eq!(cmd.get_program(), "powershell");
            assert_eq!(
                args,
                vec![
                    std::ffi::OsStr::new("-NoProfile"),
                    std::ffi::OsStr::new("-ExecutionPolicy"),
                    std::ffi::OsStr::new("Bypass"),
                    std::ffi::OsStr::new("-File"),
                    script.as_os_str(),
                ]
            );
        }
    }
}
