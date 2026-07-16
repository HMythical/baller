use std::path::PathBuf;
use std::{
    fs::{File, OpenOptions},
    io::Read,
};

use crate::error::error::BallError;
use crate::platform::common::PlatformManager;

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub source_order: Vec<String>,
    pub baller_registry_url: String,
    pub chocolatey_feed_url: String,
    pub github_enabled: bool,
    pub baller_enabled: bool,
    pub chocolatey_enabled: bool,
    pub system_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct HooksConfig {
    pub pre_install: bool,
    pub post_install: bool,
    pub pre_eject: bool,
    pub post_eject: bool,
    pub pre_update: bool,
    pub post_update: bool,
}

#[derive(Debug, Clone)]
pub struct BallerConfig {
    pub install_dir: PathBuf,
    pub db_path: PathBuf,
    pub cache_dir: PathBuf,
    pub hooks_dir: PathBuf,
    pub registry: RegistryConfig,
    pub hooks: HooksConfig,
}

impl BallerConfig {
    fn default() -> Self {
        let install_dir =
            ActiveManager::get_install_dir().unwrap_or_else(|_| PathBuf::from("/usr/local/bin"));
        let config_dir = ActiveManager::get_config_dir().unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".baller")
        });

        let mut db_path = config_dir.clone();
        db_path.push("db");
        db_path.push("baller.db");

        let mut cache_dir = config_dir.clone();
        cache_dir.push("cache");

        let mut hooks_dir = config_dir.clone();
        hooks_dir.push("hooks");

        Self {
            install_dir,
            db_path,
            cache_dir,
            hooks_dir,
            registry: RegistryConfig {
                source_order: vec![
                    "github".to_string(),
                    "baller".to_string(),
                    "chocolatey".to_string(),
                ],
                baller_registry_url: "https://registry.baller.dev/api".to_string(),
                chocolatey_feed_url: "https://community.chocolatey.org/api/v2".to_string(),
                github_enabled: true,
                baller_enabled: true,
                chocolatey_enabled: true,
                system_enabled: true,
            },
            hooks: HooksConfig {
                pre_install: true,
                post_install: true,
                pre_eject: true,
                post_eject: true,
                pre_update: true,
                post_update: true,
            },
        }
    }

    #[allow(clippy::suspicious_open_options)]
    pub fn parse_config(baller_path: &str) -> Result<BallerConfig, BallError> {
        let mut config = BallerConfig::default();
        let config_path = format!("{}/baller.conf", baller_path);

        let mut config_file: File = OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .open(&config_path)
            .map_err(BallError::FileIoErr)?;

        let mut config_content = String::new();
        config_file
            .read_to_string(&mut config_content)
            .map_err(BallError::FileIoErr)?;

        for (line, raw_line) in config_content.lines().enumerate() {
            let trimmed = raw_line.trim();

            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }

            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let section = trimmed[1..trimmed.len() - 1].trim();
                match section.to_lowercase().as_str() {
                    "baller" | "registry" | "hooks" => {}
                    _ => {
                        return Err(BallError::UnknownConfigEntry((
                            line + 1,
                            section.to_string(),
                        )));
                    }
                }
                continue;
            }

            let eq_pos = trimmed.find('=').ok_or_else(|| {
                BallError::InvalidConfig(format!(
                    "invalid config at line[{}]: expected 'key = value'",
                    line + 1
                ))
            })?;

            let key = trimmed[..eq_pos].trim();
            let value = trimmed[eq_pos + 1..].trim().trim_matches('"');

            if key.is_empty() {
                return Err(BallError::InvalidConfig(format!(
                    "invalid config at line[{}]: empty key",
                    line + 1
                )));
            }

            match key {
                "install_dir" => {
                    config.install_dir = PathBuf::from(value);
                }
                "db_path" => {
                    config.db_path = PathBuf::from(value);
                }
                "cache_dir" => {
                    config.cache_dir = PathBuf::from(value);
                }
                "source_order" => {
                    config.registry.source_order = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                "baller_registry_url" => {
                    config.registry.baller_registry_url = value.to_string();
                }
                "chocolatey_feed_url" => {
                    config.registry.chocolatey_feed_url = value.to_string();
                }
                "github_enabled" => {
                    config.registry.github_enabled = parse_bool(value, line)?;
                }
                "baller_enabled" => {
                    config.registry.baller_enabled = parse_bool(value, line)?;
                }
                "chocolatey_enabled" => {
                    config.registry.chocolatey_enabled = parse_bool(value, line)?;
                }
                "system_enabled" => {
                    config.registry.system_enabled = parse_bool(value, line)?;
                }
                "pre_install" => {
                    config.hooks.pre_install = parse_bool(value, line)?;
                }
                "post_install" => {
                    config.hooks.post_install = parse_bool(value, line)?;
                }
                "pre_eject" => {
                    config.hooks.pre_eject = parse_bool(value, line)?;
                }
                "post_eject" => {
                    config.hooks.post_eject = parse_bool(value, line)?;
                }
                "pre_update" => {
                    config.hooks.pre_update = parse_bool(value, line)?;
                }
                "post_update" => {
                    config.hooks.post_update = parse_bool(value, line)?;
                }
                _ => {
                    return Err(BallError::UnknownConfigEntry((line + 1, key.to_string())));
                }
            }
        }

        Ok(config)
    }
}

fn parse_bool(value: &str, line: usize) -> Result<bool, BallError> {
    match value.to_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        _ => Err(BallError::InvalidConfig(format!(
            "invalid boolean '{}' at line[{}]: expected true/false, yes/no, 1/0, or on/off",
            value,
            line + 1
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bool_true_variants() {
        assert_eq!(parse_bool("true", 0).unwrap(), true);
        assert_eq!(parse_bool("yes", 0).unwrap(), true);
        assert_eq!(parse_bool("1", 0).unwrap(), true);
        assert_eq!(parse_bool("on", 0).unwrap(), true);
        assert_eq!(parse_bool("TRUE", 0).unwrap(), true);
        assert_eq!(parse_bool("YES", 0).unwrap(), true);
        assert_eq!(parse_bool("ON", 0).unwrap(), true);
    }

    #[test]
    fn test_parse_bool_false_variants() {
        assert_eq!(parse_bool("false", 0).unwrap(), false);
        assert_eq!(parse_bool("no", 0).unwrap(), false);
        assert_eq!(parse_bool("0", 0).unwrap(), false);
        assert_eq!(parse_bool("off", 0).unwrap(), false);
        assert_eq!(parse_bool("FALSE", 0).unwrap(), false);
        assert_eq!(parse_bool("OFF", 0).unwrap(), false);
    }

    #[test]
    fn test_parse_bool_invalid() {
        let result = parse_bool("maybe", 2);
        assert!(result.is_err());
        match result.unwrap_err() {
            BallError::InvalidConfig(msg) => {
                assert!(msg.contains("maybe"));
                assert!(msg.contains("line[3]"));
            }
            _ => panic!("expected InvalidConfig"),
        }
    }

    fn write_config(content: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir()
            .join("baller_test_config")
            .join(uuid_like());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("baller.conf"), content).unwrap();
        let path = dir.to_string_lossy().to_string();
        (path, dir)
    }

    fn uuid_like() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("config_test_{}", nanos)
    }

    #[test]
    fn test_parse_config_empty() {
        let (path, dir) = write_config("");
        let config = BallerConfig::parse_config(&path).unwrap();
        assert!(
            config.install_dir.to_string_lossy().contains("local")
                || config.install_dir.to_string_lossy().contains("bin")
        );
        assert!(config.hooks.pre_install);
        assert!(config.registry.github_enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_comment_lines() {
        let content =
            "# this is a comment\n; also a comment\n[baller]\ninstall_dir = /custom/path\n";
        let (path, dir) = write_config(content);
        let config = BallerConfig::parse_config(&path).unwrap();
        assert_eq!(config.install_dir.to_string_lossy(), "/custom/path");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_with_all_sections() {
        let content = r#"
[baller]
install_dir = /opt/baller
db_path = /opt/baller/db/baller.db
cache_dir = /opt/baller/cache

[registry]
source_order = github, baller
baller_registry_url = https://custom.registry.com/api
chocolatey_feed_url = https://custom.chocolatey.org/feed
github_enabled = true
baller_enabled = false
chocolatey_enabled = false

[hooks]
pre_install = yes
post_install = no
pre_eject = 1
post_eject = 0
pre_update = on
post_update = off
"#;
        let (path, dir) = write_config(content);
        let config = BallerConfig::parse_config(&path).unwrap();

        assert_eq!(config.install_dir.to_string_lossy(), "/opt/baller");
        assert_eq!(config.db_path.to_string_lossy(), "/opt/baller/db/baller.db");
        assert_eq!(config.cache_dir.to_string_lossy(), "/opt/baller/cache");
        assert_eq!(
            config.registry.source_order,
            vec!["github".to_string(), "baller".to_string()]
        );
        assert_eq!(
            config.registry.baller_registry_url,
            "https://custom.registry.com/api"
        );
        assert_eq!(config.registry.github_enabled, true);
        assert_eq!(config.registry.baller_enabled, false);
        assert_eq!(config.registry.chocolatey_enabled, false);
        assert!(config.hooks.pre_install);
        assert!(!config.hooks.post_install);
        assert!(config.hooks.pre_eject);
        assert!(!config.hooks.post_eject);
        assert!(config.hooks.pre_update);
        assert!(!config.hooks.post_update);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_unknown_key() {
        let content = "unknown_key = value\n";
        let (path, dir) = write_config(content);
        let result = BallerConfig::parse_config(&path);
        assert!(result.is_err());
        match result.unwrap_err() {
            BallError::UnknownConfigEntry((line, key)) => {
                assert_eq!(line, 1);
                assert_eq!(key, "unknown_key");
            }
            _ => panic!("expected UnknownConfigEntry"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_unknown_section() {
        let content = "[unknown_section]\nkey = val\n";
        let (path, dir) = write_config(content);
        let result = BallerConfig::parse_config(&path);
        assert!(result.is_err());
        match result.unwrap_err() {
            BallError::UnknownConfigEntry((_line, section)) => {
                assert_eq!(section, "unknown_section");
            }
            _ => panic!("expected UnknownConfigEntry"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_invalid_bool() {
        let content = "[hooks]\npre_install = maybe\n";
        let (path, dir) = write_config(content);
        let result = BallerConfig::parse_config(&path);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_invalid_line_no_eq() {
        let content = "just a line without equals\n";
        let (path, dir) = write_config(content);
        let result = BallerConfig::parse_config(&path);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_empty_key() {
        let content = " = value\n";
        let (path, dir) = write_config(content);
        let result = BallerConfig::parse_config(&path);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_struct_debug() {
        let cfg = BallerConfig::default();
        let debug = format!("{:?}", cfg);
        assert!(debug.contains("BallerConfig"));
    }

    #[test]
    fn test_registry_config_debug() {
        let cfg = RegistryConfig {
            source_order: vec!["github".to_string()],
            baller_registry_url: "url".to_string(),
            chocolatey_feed_url: "feed".to_string(),
            github_enabled: true,
            baller_enabled: false,
            chocolatey_enabled: false,
            system_enabled: true,
        };
        let debug = format!("{:?}", cfg);
        assert!(debug.contains("RegistryConfig"));
    }

    #[test]
    fn test_hooks_config_default() {
        let h = HooksConfig {
            pre_install: true,
            post_install: true,
            pre_eject: true,
            post_eject: true,
            pre_update: true,
            post_update: true,
        };
        assert!(h.pre_install);
        assert!(h.post_update);
    }

    #[test]
    fn test_config_default_creates_paths() {
        let cfg = BallerConfig::default();
        assert!(!cfg.install_dir.as_os_str().is_empty());
        assert!(!cfg.db_path.as_os_str().is_empty());
        assert!(!cfg.cache_dir.as_os_str().is_empty());
        assert!(!cfg.hooks_dir.as_os_str().is_empty());
    }

    #[test]
    fn test_parse_config_quoted_values() {
        let content = r#"install_dir = "/opt/baller with spaces""#;
        let (path, dir) = write_config(content);
        let config = BallerConfig::parse_config(&path).unwrap();
        assert_eq!(
            config.install_dir.to_string_lossy(),
            "/opt/baller with spaces"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_system_enabled_default_true() {
        let cfg = BallerConfig::default();
        assert!(cfg.registry.system_enabled);
    }

    #[test]
    fn test_parse_config_system_enabled_false() {
        let content = "[registry]\nsystem_enabled = false\n";
        let (path, dir) = write_config(content);
        let config = BallerConfig::parse_config(&path).unwrap();
        assert!(!config.registry.system_enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_system_enabled_true() {
        let content = "[registry]\nsystem_enabled = true\n";
        let (path, dir) = write_config(content);
        let config = BallerConfig::parse_config(&path).unwrap();
        assert!(config.registry.system_enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_source_order_with_system() {
        let content = "[registry]\nsource_order = github,baller,chocolatey,system\n";
        let (path, dir) = write_config(content);
        let config = BallerConfig::parse_config(&path).unwrap();
        assert!(config.registry.source_order.contains(&"system".to_string()));
        assert_eq!(config.registry.source_order.len(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_system_enabled_aliases() {
        for (val, expected) in &[("yes", true), ("0", false), ("1", true), ("off", false), ("ON", true)] {
            let content = format!("[registry]\nsystem_enabled = {}\n", val);
            let (path, dir) = write_config(&content);
            let config = BallerConfig::parse_config(&path).unwrap();
            assert_eq!(config.registry.system_enabled, *expected, "system_enabled={}", val);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
