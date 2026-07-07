mod cli;
mod commands;
mod config;
mod context;
mod core;
mod error;
mod http;
mod platform;
mod utils;

use std::{
    env::{self, home_dir},
    path::PathBuf,
    process::exit,
};

use crate::{
    cli::parse::BallerCommand, config::config::BallerConfig, context::AppContext,
    error::error::BallError, utils::fs::ensure_dir,
};

pub const CRATE_VERSION: &str = "v0.1";

fn main() {
    if let Err(e) = entry() {
        eprintln!("\n[Error]: {}", e);
        exit(1);
    }
}

fn entry() -> Result<(), BallError> {
    if !cfg!(target_os = "linux") && !cfg!(target_os = "windows") {
        return Err(BallError::UnsupportedOs(env::consts::OS.to_string()));
    }

    let baller_dir = create_baller_dir()?;
    let baller_config: BallerConfig = BallerConfig::parse_config(&baller_dir)?;

    let ctx = AppContext::new(baller_config)?;

    let command: BallerCommand = BallerCommand::parse_command()?;

    command.execute(&ctx)?;

    Ok(())
}

fn create_baller_dir() -> Result<String, BallError> {
    let home_path: PathBuf = home_dir().unwrap();
    let mut baller_dir = home_path.display().to_string();

    if cfg!(target_os = "linux") {
        baller_dir.push_str("/.baller");
    } else if cfg!(target_os = "windows") {
        baller_dir.push_str("/AppData/Local/baller");
    }

    ensure_dir(baller_dir.as_ref())?;

    Ok(baller_dir)
}

#[cfg(test)]
mod integration_tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::config::config::BallerConfig;
    use crate::core::db::DbManager;
    use crate::core::dep_solver::{
        detect_cycles, parse_dependency_line, topological_sort, Dependency,
    };
    use crate::core::manifest::ManifestParser;
    use crate::core::package::{Package, PackageSource};
    use crate::error::error::BallError;
    use crate::utils::fs;
    use crate::utils::security;

    fn unique_id() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("int_{}_{}", n, std::process::id())
    }

    fn test_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("baller_integration")
            .join(unique_id());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_full_db_lifecycle_with_config() {
        let dir = test_dir();
        let config_path = dir.join("baller.conf");
        std::fs::write(&config_path, "[baller]\ninstall_dir = /tmp/baller_test\n").unwrap();

        let config_str = dir.to_string_lossy().to_string();
        let config = BallerConfig::parse_config(&config_str).unwrap();
        assert!(
            config.install_dir.to_string_lossy().contains("baller_test")
                || config.install_dir.to_string_lossy() == "/tmp/baller_test"
        );

        let db_path = dir.join("test.db");
        let db = DbManager::init_at_path(&db_path).unwrap();
        assert_eq!(db.package_count().unwrap(), 0);

        let pkg = Package::new("integration-pkg", "1.0.0");
        db.insert_package(&pkg, "/install/path", Some("/bin/path"), None)
            .unwrap();
        assert_eq!(db.package_count().unwrap(), 1);

        let retrieved = db.get_package("integration-pkg").unwrap();
        assert_eq!(retrieved.version, "1.0.0");

        db.set_frozen("integration-pkg", true).unwrap();
        assert!(db.is_frozen("integration-pkg").unwrap());

        db.remove_package("integration-pkg").unwrap();
        assert_eq!(db.package_count().unwrap(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_manifest_toml_json_roundtrip() {
        let pkg = Package {
            name: "roundtrip-pkg".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Round trip test".to_string()),
            author: Some("Author".to_string()),
            repository: Some("https://github.com/user/repo".to_string()),
            architectures: None,
            dependencies: Some(vec!["dep1".to_string(), "dep2 >=2.0".to_string()]),
            sha256: Some("abc".to_string()),
            download_url: Some("https://example.com/pkg.tar.gz".to_string()),
            source: PackageSource::GitHub {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
            },
        };

        let toml_str = toml::to_string(&pkg).unwrap();
        let from_toml = ManifestParser::parse_toml(&toml_str).unwrap();
        assert_eq!(from_toml.name, "roundtrip-pkg");
        assert_eq!(from_toml.dependencies.as_ref().unwrap().len(), 2);

        let json_str = serde_json::to_string(&pkg).unwrap();
        let from_json = ManifestParser::parse_json(&json_str).unwrap();
        assert_eq!(from_json.name, "roundtrip-pkg");
        assert_eq!(from_json.version, "1.0.0");
    }

    #[test]
    fn test_dep_solver_graph_operations() {
        let mut graph = std::collections::HashMap::new();
        graph.insert("root".to_string(), vec!["a".to_string(), "b".to_string()]);
        graph.insert("a".to_string(), vec!["c".to_string()]);
        graph.insert("b".to_string(), vec!["c".to_string()]);
        graph.insert("c".to_string(), vec![]);

        assert!(detect_cycles(&graph).is_ok());
        let order = topological_sort(&graph).unwrap();
        assert_eq!(order.len(), 4);
        assert!(order.iter().position(|n| n == "c") < order.iter().position(|n| n == "a"));
        assert!(order.iter().position(|n| n == "c") < order.iter().position(|n| n == "b"));
    }

    #[test]
    fn test_dep_solver_cycle_detection() {
        let mut graph = std::collections::HashMap::new();
        graph.insert("x".to_string(), vec!["y".to_string()]);
        graph.insert("y".to_string(), vec!["z".to_string()]);
        graph.insert("z".to_string(), vec!["x".to_string()]);
        assert!(detect_cycles(&graph).is_err());
    }

    #[test]
    fn test_dependency_parsing_edge_cases() {
        let dep = parse_dependency_line("?");
        assert_eq!(dep.name, "");
        assert!(dep.optional);

        let dep = parse_dependency_line("");
        assert_eq!(dep.name, "");

        let dep = parse_dependency_line("pkg >=1.0.0-alpha.1");
        assert_eq!(dep.name, "pkg");
        assert!(!dep.optional);
    }

    #[test]
    fn test_utils_security_fs_integration() {
        let dir = test_dir();
        let file_path = dir.join("checksum.txt");
        let content = b"integration test content for checksum verification";
        std::fs::write(&file_path, content).unwrap();

        let hash = security::sha256_file(&file_path).unwrap();
        assert_eq!(hash.len(), 64);

        security::verify_checksum(&file_path, &hash).unwrap();

        let wrong_hash = "a".repeat(64);
        let result = security::verify_checksum(&file_path, &wrong_hash);
        assert!(result.is_err());

        let dir_size = fs::dir_size(&dir);
        assert!(dir_size > 0);

        let sanitized = fs::sanitize_filename("hello:world?test=1&2");
        assert_eq!(sanitized, "hello_world_test_1_2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_utils_symlink_integration() {
        let dir = test_dir();
        let source = dir.join("source_binary");
        let link = dir.join("linked_binary");

        std::fs::write(&source, b"binary content").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(&source, &link).unwrap();
            assert!(link.exists());
            assert!(link.is_symlink() || link.exists());
            std::fs::remove_file(&link).unwrap();
        }

        #[cfg(not(unix))]
        {
            use std::os::windows::fs::symlink_file;
            if symlink_file(&source, &link).is_ok() {
                assert!(link.exists());
                std::fs::remove_file(&link).unwrap();
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_parse_with_boolean_aliases() {
        let dir = test_dir();
        let config_path = dir.join("baller.conf");
        let content = r#"
[hooks]
pre_install = yes
post_install = no
pre_eject = 1
post_eject = 0
pre_update = on
post_update = off
"#;
        std::fs::write(&config_path, content).unwrap();
        let path = dir.to_string_lossy().to_string();
        let config = BallerConfig::parse_config(&path).unwrap();
        assert!(config.hooks.pre_install);
        assert!(!config.hooks.post_install);
        assert!(config.hooks.pre_eject);
        assert!(!config.hooks.post_eject);
        assert!(config.hooks.pre_update);
        assert!(!config.hooks.post_update);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_error_cross_module_consistency() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "test");
        let ball_err = BallError::FileIoErr(io_err);
        let msg = format!("{}", ball_err);
        assert!(msg.contains("test"));

        let network_err = BallError::NetworkError("connection refused".to_string());
        let msg = format!("{}", network_err);
        assert!(msg.contains("connection refused"));

        let pkg_err = BallError::PackageNotFound("missing".to_string());
        let msg = format!("{}", pkg_err);
        assert!(msg.contains("missing"));
    }

    #[test]
    fn test_package_serialization_across_formats() {
        let pkg = Package::new("cross-format", "1.0.0");

        let toml = toml::to_string(&pkg).unwrap();
        let from_toml: Package = toml::from_str(&toml).unwrap();
        assert_eq!(from_toml.name, "cross-format");

        let json = serde_json::to_string(&pkg).unwrap();
        let from_json: Package = serde_json::from_str(&json).unwrap();
        assert_eq!(from_json.name, "cross-format");

        assert_eq!(from_toml.name, from_json.name);
        assert_eq!(from_toml.version, from_json.version);
    }

    #[test]
    fn test_registry_source_matching() {
        use crate::core::registry::RegistrySource;

        let sources = vec![
            RegistrySource::GitHub,
            RegistrySource::BallerRegistry,
            RegistrySource::Chocolatey,
        ];

        assert_eq!(sources.len(), 3);
        assert!(sources.contains(&RegistrySource::GitHub));
        assert!(sources.contains(&RegistrySource::Chocolatey));

        for s in &sources {
            match s {
                RegistrySource::GitHub
                | RegistrySource::BallerRegistry
                | RegistrySource::Chocolatey => {}
            }
        }
    }

    #[test]
    fn test_hook_type_matching() {
        use crate::core::hooks::HookType;

        let pre = HookType::PreInstall;
        let post = HookType::PostInstall;

        assert_ne!(pre, post);
        assert_eq!(pre.type_str(), "pre_install");
        assert_eq!(post.type_str(), "post_install");
    }

    #[test]
    fn test_dependency_struct_usage() {
        use semver::VersionReq;

        let dep = Dependency {
            name: "test-dep".to_string(),
            constraint: VersionReq::parse(">=1.0").unwrap(),
            optional: false,
        };
        assert_eq!(dep.name, "test-dep");
        assert!(!dep.optional);
    }

    #[test]
    fn test_fs_ensure_atomic_integration() {
        let dir = test_dir();
        let file = dir.join("atomic_test.txt");

        fs::atomic_write(&file, b"atomic content").unwrap();
        assert!(file.exists());

        let content = std::fs::read_to_string(&file).unwrap();
        assert_eq!(content, "atomic content");

        let copied = dir.join("copied.txt");
        fs::copy_file(&file, &copied).unwrap();
        assert!(copied.exists());
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), "atomic content");

        fs::remove_file(&copied).unwrap();
        assert!(!copied.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fs_sanitize_url_as_filename() {
        let url = "https://github.com/owner/repo/releases/download/v1.0.0/package.tar.gz";
        let sanitized = fs::sanitize_filename(url);
        assert!(!sanitized.contains('/'));
        assert!(!sanitized.contains(':'));
        assert!(sanitized.contains("github.com"));
        assert!(sanitized.ends_with(".tar.gz") || sanitized.ends_with(".gz"));
    }
}
