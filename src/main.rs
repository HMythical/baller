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
    cli::parse::BallerCommand,
    config::config::{BallerConfig, HooksConfig},
    context::AppContext,
    error::error::BallError,
    utils::fs::ensure_dir,
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

    let command: BallerCommand = BallerCommand::parse_command()?;

    if command.no_color {
        colored::control::set_override(false);
    }

    let baller_dir = resolve_baller_dir(command.config.as_deref())?;

    // A custom --config directory takes the db, cache and hooks with it.
    let mut baller_config: BallerConfig = match command.config {
        Some(_) => BallerConfig::parse_config_rooted(&baller_dir, true)?,
        None => BallerConfig::parse_config(&baller_dir)?,
    };

    if command.no_hooks {
        baller_config.hooks = HooksConfig::disabled();
    }

    let ctx = AppContext::new(baller_config, command.global_flags())?;

    command.execute(&ctx)?;

    Ok(())
}

/// The baller directory to work out of: `--config <dir>` when given, otherwise
/// the platform default. Everything else (db, cache, hooks) derives from it.
fn resolve_baller_dir(config_override: Option<&str>) -> Result<String, BallError> {
    match config_override {
        Some(dir) => {
            ensure_dir(dir.as_ref())?;
            Ok(dir.to_string())
        }
        None => create_baller_dir(),
    }
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

    use crate::config::config::{BallerConfig, RegistryConfig};
    use crate::context::effective_source_order;
    use crate::core::db::DbManager;
    use crate::core::dep_solver::{
        detect_cycles, parse_dependency_line, topological_sort, Dependency,
    };
    use crate::core::downloader::Downloader;
    use crate::core::manifest::ManifestParser;
    use crate::core::package::{Package, PackageSource};
    use crate::core::registry::RegistrySource;
    use crate::error::error::BallError;
    use crate::http::HttpClient;
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
        db.insert_package(&pkg, "/install/path", Some("/bin/path"), None, true)
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
            hash_algorithm: None,
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
            RegistrySource::System,
            RegistrySource::Cargo,
        ];

        assert_eq!(sources.len(), 5);
        assert!(sources.contains(&RegistrySource::GitHub));
        assert!(sources.contains(&RegistrySource::Chocolatey));
        assert!(sources.contains(&RegistrySource::System));
        assert!(sources.contains(&RegistrySource::Cargo));

        for s in &sources {
            match s {
                RegistrySource::GitHub
                | RegistrySource::BallerRegistry
                | RegistrySource::Chocolatey
                | RegistrySource::System
                | RegistrySource::Cargo => {}
            }
        }
    }

    #[test]
    fn test_package_manager_error_display() {
        let err = BallError::PackageManagerError("no package manager detected".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("package manager error"));
        assert!(msg.contains("no package manager detected"));

        let debug = format!("{:?}", err);
        assert!(debug.contains("PackageManagerError"));
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

    fn registry_config(source_order: &[&str]) -> RegistryConfig {
        RegistryConfig {
            source_order: source_order.iter().map(|s| s.to_string()).collect(),
            baller_registry_url: "https://registry.baller.dev/api".to_string(),
            chocolatey_feed_url: "https://community.chocolatey.org/api/v2".to_string(),
            github_default_owner: None,
            github_enabled: true,
            baller_enabled: true,
            chocolatey_enabled: true,
            system_enabled: true,
            cargo_enabled: true,
        }
    }

    #[test]
    fn test_effective_order_excludes_disabled_system() {
        let mut config = registry_config(&["github", "system"]);
        config.system_enabled = false;

        let effective_order = effective_source_order(&config);

        assert_eq!(effective_order.len(), 1);
        assert!(effective_order.contains(&RegistrySource::GitHub));
        assert!(!effective_order.contains(&RegistrySource::System));
    }

    #[test]
    fn test_effective_order_includes_enabled_system() {
        let config = registry_config(&["github", "system"]);

        let effective_order = effective_source_order(&config);

        assert_eq!(effective_order.len(), 2);
        assert!(effective_order.contains(&RegistrySource::GitHub));
        assert!(effective_order.contains(&RegistrySource::System));
    }

    #[test]
    fn test_effective_order_excludes_disabled_cargo() {
        let mut config = registry_config(&["cargo", "github"]);
        config.cargo_enabled = false;

        let effective_order = effective_source_order(&config);

        assert_eq!(effective_order, vec![RegistrySource::GitHub]);
        assert!(!effective_order.contains(&RegistrySource::Cargo));
    }

    #[test]
    fn test_effective_order_includes_enabled_cargo() {
        let config = registry_config(&["baller", "system", "cargo", "github"]);

        let effective_order = effective_source_order(&config);

        assert_eq!(
            effective_order,
            vec![
                RegistrySource::BallerRegistry,
                RegistrySource::System,
                RegistrySource::Cargo,
                RegistrySource::GitHub
            ]
        );
    }

    #[test]
    fn test_effective_order_preserves_configured_order() {
        let config = registry_config(&["baller", "chocolatey", "github"]);

        let effective_order = effective_source_order(&config);

        assert_eq!(
            effective_order,
            vec![
                RegistrySource::BallerRegistry,
                RegistrySource::Chocolatey,
                RegistrySource::GitHub
            ]
        );
    }

    #[test]
    fn test_effective_order_drops_unknown_sources() {
        let config = registry_config(&["npm", "baller", "github"]);

        let effective_order = effective_source_order(&config);

        assert_eq!(
            effective_order,
            vec![RegistrySource::BallerRegistry, RegistrySource::GitHub]
        );
    }

    #[test]
    fn test_effective_order_from_platform_defaults() {
        let config = BallerConfig::default();

        let effective_order = effective_source_order(&config.registry);

        assert_eq!(
            effective_order.first(),
            Some(&RegistrySource::BallerRegistry)
        );
        assert_eq!(effective_order.last(), Some(&RegistrySource::GitHub));

        if cfg!(target_os = "windows") {
            assert_eq!(
                effective_order,
                vec![
                    RegistrySource::BallerRegistry,
                    RegistrySource::Chocolatey,
                    RegistrySource::GitHub
                ]
            );
        } else {
            assert_eq!(
                effective_order,
                vec![
                    RegistrySource::BallerRegistry,
                    RegistrySource::System,
                    RegistrySource::Cargo,
                    RegistrySource::GitHub
                ]
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_windows_defaults_skip_system_source() {
        let config = BallerConfig::default();
        assert!(!config.registry.system_enabled);

        let effective_order = effective_source_order(&config.registry);
        assert!(!effective_order.contains(&RegistrySource::System));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_defaults_skip_chocolatey_source() {
        let config = BallerConfig::default();
        assert!(!config.registry.chocolatey_enabled);

        let effective_order = effective_source_order(&config.registry);
        assert!(!effective_order.contains(&RegistrySource::Chocolatey));
    }

    #[test]
    fn test_github_stays_available_as_fallback() {
        let config = BallerConfig::default();
        let effective_order = effective_source_order(&config.registry);

        assert!(config.registry.github_enabled);
        assert!(effective_order.contains(&RegistrySource::GitHub));
        assert_eq!(effective_order.last(), Some(&RegistrySource::GitHub));
    }

    #[test]
    fn test_build_manifest_fixtures_parse_and_validate() {
        let dir = test_dir();

        let flat_toml = dir.join("flat.toml");
        std::fs::write(
            &flat_toml,
            r#"name = "flat-pkg"
version = "1.0.0"
dependencies = ["libc >=0.2.0"]
sha256 = "deadbeef"
download_url = "https://example.com/flat-pkg.tar.gz"

[source]
GitHub = { owner = "owner", repo = "flat-pkg" }
"#,
        )
        .unwrap();

        let nested_toml = dir.join("baller.toml");
        std::fs::write(
            &nested_toml,
            r#"name = "nested-pkg"
version = "2.0.0"
repository = "https://github.com/owner/nested-pkg"

[source]
type = "github"
owner = "owner"
repo = "nested-pkg"

[dependencies]
"libc" = ">=0.2.0"
"?extra" = "*"

[architectures]
supported = ["x86_64"]

[checksum]
sha256 = "cafebabe"
"#,
        )
        .unwrap();

        let flat_json = dir.join("flat.json");
        std::fs::write(
            &flat_json,
            r#"{"name": "flat-json-pkg", "version": "3.0.0", "sha256": "abc123"}"#,
        )
        .unwrap();

        let nested_json = dir.join("baller.json");
        std::fs::write(
            &nested_json,
            r#"{
    "name": "nested-json-pkg",
    "version": "4.0.0",
    "source": { "type": "chocolatey", "feed_url": "https://feed.example.com/api/v2" },
    "checksum": { "sha256": "beefcafe" }
}"#,
        )
        .unwrap();

        for path in [&flat_toml, &nested_toml, &flat_json, &nested_json] {
            let pkg = ManifestParser::parse_auto(path).unwrap();
            ManifestParser::validate(&pkg).unwrap();
            assert!(!pkg.name.is_empty());
            assert!(pkg.sha256.is_some());
        }

        let nested = ManifestParser::parse_auto(&nested_toml).unwrap();
        assert_eq!(nested.version, "2.0.0");
        assert_eq!(nested.architectures.unwrap(), vec!["x86_64".to_string()]);
        assert_eq!(nested.dependencies.unwrap().len(), 2);
        assert_eq!(
            nested.source,
            PackageSource::GitHub {
                owner: "owner".to_string(),
                repo: "nested-pkg".to_string()
            }
        );

        let nested_json_pkg = ManifestParser::parse_auto(&nested_json).unwrap();
        assert_eq!(
            nested_json_pkg.source,
            PackageSource::Chocolatey {
                feed_url: "https://feed.example.com/api/v2".to_string()
            }
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_build_manifest_error_paths() {
        let dir = test_dir();

        let missing = dir.join("nope.toml");
        match ManifestParser::parse_auto(&missing) {
            Err(BallError::InvalidConfig(msg)) => assert!(msg.contains("not found")),
            other => panic!("expected InvalidConfig, got {:?}", other),
        }

        let no_version = dir.join("no-version.toml");
        std::fs::write(&no_version, "name = \"pkg\"\n").unwrap();
        assert!(ManifestParser::parse_auto(&no_version).is_err());

        let empty_version = dir.join("empty-version.json");
        std::fs::write(&empty_version, r#"{"name": "pkg", "version": ""}"#).unwrap();
        let pkg = ManifestParser::parse_auto(&empty_version).unwrap();
        match ManifestParser::validate(&pkg) {
            Err(BallError::InvalidConfig(msg)) => assert!(msg.contains("version")),
            other => panic!("expected InvalidConfig, got {:?}", other),
        }

        let bad_source = dir.join("bad-source.toml");
        std::fs::write(
            &bad_source,
            "name = \"pkg\"\nversion = \"1.0.0\"\n\n[source]\ntype = \"npm\"\n",
        )
        .unwrap();
        assert!(ManifestParser::parse_auto(&bad_source).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_build_assembles_from_local_archive() {
        use std::io::Write;

        let dir = test_dir();
        let cache_dir = dir.join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let binary_name = if cfg!(target_os = "windows") {
            "local-pkg.exe"
        } else {
            "local-pkg"
        };

        let archive_path = dir.join("local-pkg.zip");
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file(binary_name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"binary contents").unwrap();
            zip.finish().unwrap();
        }

        let manifest_path = dir.join("baller.toml");
        std::fs::write(
            &manifest_path,
            r#"name = "local-pkg"
version = "1.2.3"
description = "assembled from a local archive"

[source]
type = "github"
owner = "owner"
repo = "local-pkg"
"#,
        )
        .unwrap();

        let pkg = ManifestParser::parse_auto(&manifest_path).unwrap();
        ManifestParser::validate(&pkg).unwrap();

        let downloader = Downloader::new(cache_dir.clone(), HttpClient::new().unwrap());
        let extract_dir = cache_dir.join(format!("{}-{}", pkg.name, pkg.version));
        std::fs::create_dir_all(&extract_dir).unwrap();
        downloader
            .extract_archive(&archive_path, &extract_dir)
            .unwrap();

        let binary_path = fs::find_binary_in_dir(&extract_dir, &pkg.name).unwrap();
        assert_eq!(binary_path.file_name().unwrap(), binary_name);

        let db = DbManager::init_at_path(&dir.join("build.db")).unwrap();
        let manifest_str = manifest_path.to_string_lossy().to_string();
        db.insert_package(
            &pkg,
            &extract_dir.to_string_lossy(),
            Some(&binary_path.to_string_lossy()),
            Some(&manifest_str),
            true,
        )
        .unwrap();

        let recorded = db.get_package("local-pkg").unwrap();
        assert_eq!(recorded.version, "1.2.3");
        assert!(recorded.user_installed);
        assert_eq!(recorded.manifest_path, Some(manifest_str));
        assert_eq!(recorded.install_path, extract_dir.to_string_lossy());
        assert_eq!(recorded.source, "github");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
