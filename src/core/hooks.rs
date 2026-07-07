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

    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = Command::new("bash");
        c.arg(&script_path);
        c
    };

    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("powershell");
        c.arg("-File").arg(&script_path);
        c
    };

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
        return Err(BallError::InvalidConfig(format!(
            "hook '{}' failed with exit code {:?}",
            script_name,
            status.code()
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
}
