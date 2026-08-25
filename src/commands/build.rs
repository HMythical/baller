use colored::Colorize;
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::commands::draft::source_label;
use crate::context::AppContext;
use crate::core::hooks::{run_hook, HookType};
use crate::core::manifest::{parse_github_url, ManifestParser};
use crate::core::package::{Package, PackageSource};
use crate::core::registry::RegistrySource;
use crate::error::error::BallError;
use crate::http::chocolatey::ChocolateyRegistry;
use crate::http::github::GitHubRegistry;
use crate::http::registry_api::BallerRegistryApi;
use crate::http::system::install_system_package;
use crate::platform::common::PlatformManager;
use crate::utils::output::{info, print_json};

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

const MANIFEST_NAMES: [&str; 2] = ["baller.toml", "baller.json"];

pub struct BuildOptions {
    pub dry_run: bool,
    pub no_deps: bool,
    pub install_dir: Option<String>,
    pub force: bool,
    pub source: Option<RegistrySource>,
}

pub fn execute_build(ctx: &AppContext, path: &str, opts: &BuildOptions) -> Result<(), BallError> {
    let quiet = ctx.flags.is_quiet();
    let manifest_path = resolve_manifest_path(path)?;
    let manifest_str = manifest_path.to_string_lossy().to_string();

    info(
        quiet,
        format!("{} {}...", "Building".green().bold(), manifest_str.cyan()),
    );

    let mut pkg = ManifestParser::parse_auto(&manifest_path)?;
    ManifestParser::validate(&pkg)?;

    if let Some(source) = &opts.source {
        override_source(ctx, &mut pkg, source)?;
    }

    if opts.no_deps {
        pkg.dependencies = None;
    }

    if opts.dry_run {
        return report_plan(ctx, &pkg, &manifest_str, opts);
    }

    if !opts.force && ctx.db.package_exists(&pkg.name)? {
        return Err(BallError::InvalidConfig(format!(
            "'{}' is already on the roster — pass --force to build over it",
            pkg.name
        )));
    }

    run_hook(
        &HookType::PreInstall,
        &pkg.name,
        &pkg.version,
        &ctx.config.hooks_dir,
        &ctx.config.hooks,
        &[],
    )?;

    if let PackageSource::System { manager } = &pkg.source {
        return build_system_package(ctx, &pkg, manager, &manifest_str);
    }

    if pkg.download_url.is_none() {
        resolve_download_url(ctx, &mut pkg)?;
    }

    let downloaded = ctx.downloader.download_and_extract(&pkg, !quiet)?;

    let install_path = downloaded.extract_dir.to_string_lossy().to_string();
    let bin_path_str = downloaded
        .binary_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    if let Some(binary_path) = &downloaded.binary_path {
        match &opts.install_dir {
            Some(dir) => {
                ActiveManager::create_symlink_in(Path::new(dir), binary_path, &pkg.name)?;
                info(
                    quiet,
                    format!("{} linked into {}", "Linked".green(), dir.cyan()),
                );
            }
            None => {
                ActiveManager::create_symlink(binary_path, &pkg.name)?;
                info(
                    quiet,
                    format!(
                        "{} symlinked to {}",
                        "Linked".green(),
                        binary_path.display().to_string().cyan()
                    ),
                );
            }
        }
    } else {
        info(
            quiet,
            format!(
                "{} no binary found in extracted package",
                "Warning".yellow()
            ),
        );
    }

    ctx.db.insert_package(
        &pkg,
        &install_path,
        bin_path_str.as_deref(),
        Some(&manifest_str),
        true,
    )?;

    let bin_path_ref = bin_path_str.as_deref().unwrap_or("");
    let post_env = [
        ("BALLER_INSTALL_PATH", install_path.as_str()),
        ("BALLER_BIN_PATH", bin_path_ref),
    ];
    run_hook(
        &HookType::PostInstall,
        &pkg.name,
        &pkg.version,
        &ctx.config.hooks_dir,
        &ctx.config.hooks,
        &post_env,
    )?;

    announce_dependencies(ctx, &pkg);

    if ctx.flags.json {
        return print_json(&json!({
            "command": "build",
            "manifest": manifest_str,
            "package": pkg.name,
            "version": pkg.version,
            "source": source_label(&pkg.source),
            "install_path": install_path,
            "bin_path": bin_path_str,
            "status": "built",
        }));
    }

    println!(
        "{} {} v{} built!",
        "Done".green().bold(),
        pkg.name.cyan(),
        pkg.version.yellow()
    );

    Ok(())
}

/// Resolve a build target to a manifest file.
///
/// A directory resolves `baller.toml` then `baller.json`; a file is taken as-is.
fn resolve_manifest_path(path: &str) -> Result<PathBuf, BallError> {
    let candidate = Path::new(path);

    if candidate.is_dir() {
        for name in MANIFEST_NAMES {
            let manifest = candidate.join(name);
            if manifest.is_file() {
                return Ok(manifest);
            }
        }

        return Err(BallError::InvalidConfig(format!(
            "no baller.toml or baller.json found in '{}'",
            candidate.display()
        )));
    }

    if candidate.is_file() {
        return Ok(candidate.to_path_buf());
    }

    Err(BallError::InvalidConfig(format!(
        "manifest not found: {}",
        candidate.display()
    )))
}

/// `--source`: replace the manifest's source, discarding a `download_url` that
/// belonged to the old one so the new source is actually consulted.
fn override_source(
    ctx: &AppContext,
    pkg: &mut Package,
    source: &RegistrySource,
) -> Result<(), BallError> {
    let replacement = match source {
        RegistrySource::GitHub => {
            let repo_url = pkg.repository.as_deref().ok_or_else(|| {
                BallError::InvalidConfig(
                    "--source github needs a github.com 'repository' URL in the manifest"
                        .to_string(),
                )
            })?;
            let (owner, repo) = parse_github_url(repo_url).ok_or_else(|| {
                BallError::InvalidConfig(format!(
                    "--source github could not read owner/repo from '{}'",
                    repo_url
                ))
            })?;
            PackageSource::GitHub { owner, repo }
        }
        RegistrySource::Chocolatey => PackageSource::Chocolatey {
            feed_url: ctx.config.registry.chocolatey_feed_url.clone(),
        },
        RegistrySource::BallerRegistry => PackageSource::BallerRegistry {
            url: ctx.config.registry.baller_registry_url.clone(),
        },
        RegistrySource::System => {
            let manager = ctx.registry.system_manager_name().ok_or_else(|| {
                BallError::UnsupportedOs(
                    "--source system needs a native package manager, which only Linux hosts have"
                        .to_string(),
                )
            })?;
            PackageSource::System {
                manager: manager.to_string(),
            }
        }
    };

    pkg.source = replacement;
    pkg.download_url = None;
    Ok(())
}

/// `--dry-run`: report the manifest as parsed, touching nothing
fn report_plan(
    ctx: &AppContext,
    pkg: &Package,
    manifest_path: &str,
    opts: &BuildOptions,
) -> Result<(), BallError> {
    let already_installed = ctx.db.package_exists(&pkg.name).unwrap_or(false);
    let deps = pkg.dependencies.clone().unwrap_or_default();

    if ctx.flags.json {
        return print_json(&json!({
            "command": "build",
            "manifest": manifest_path,
            "package": pkg.name,
            "version": pkg.version,
            "source": source_label(&pkg.source),
            "download_url": pkg.download_url,
            "dependencies": deps,
            "install_dir": opts.install_dir,
            "already_installed": already_installed,
            "dry_run": true,
        }));
    }

    println!(
        "{} {} v{} from {}",
        "Dry run".yellow().bold(),
        pkg.name.cyan(),
        pkg.version.yellow(),
        source_label(&pkg.source).cyan()
    );
    println!("  {} {}", "Manifest:".yellow(), manifest_path);
    match &pkg.download_url {
        Some(url) => println!("  {} {}", "Download:".yellow(), url),
        None => println!(
            "  {} resolved from the source at build time",
            "Download:".yellow()
        ),
    }
    if let Some(hash) = &pkg.sha256 {
        println!("  {} {}", "Checksum:".yellow(), hash);
    }
    if let Some(dir) = &opts.install_dir {
        println!("  {} {}", "Install dir:".yellow(), dir);
    }
    if deps.is_empty() {
        println!("  {} none", "Dependencies:".yellow());
    } else {
        println!("  {} {}", "Dependencies:".yellow(), deps.join(", "));
    }
    if already_installed {
        println!(
            "  {} already on the roster — needs {}",
            "Note:".yellow(),
            "--force".cyan()
        );
    }

    println!("{} nothing was installed", "Note".yellow());
    Ok(())
}

/// Install a manifest that names a system package via the native manager
fn build_system_package(
    ctx: &AppContext,
    pkg: &Package,
    manager: &str,
    manifest_path: &str,
) -> Result<(), BallError> {
    if !cfg!(target_os = "linux") {
        return Err(BallError::UnsupportedOs(format!(
            "'{}' needs the {} system package manager, which is Linux only",
            pkg.name, manager
        )));
    }

    let quiet = ctx.flags.is_quiet();

    info(
        quiet,
        format!("{} installing via {}...", "System".green(), manager.cyan()),
    );
    install_system_package(manager, &pkg.name)?;

    ctx.db
        .insert_package(pkg, "", None, Some(manifest_path), true)?;

    run_hook(
        &HookType::PostInstall,
        &pkg.name,
        &pkg.version,
        &ctx.config.hooks_dir,
        &ctx.config.hooks,
        &[],
    )?;

    announce_dependencies(ctx, pkg);

    if ctx.flags.json {
        return print_json(&json!({
            "command": "build",
            "manifest": manifest_path,
            "package": pkg.name,
            "version": pkg.version,
            "source": source_label(&pkg.source),
            "status": "installed",
        }));
    }

    println!(
        "{} {} v{} installed via {}!",
        "Done".green().bold(),
        pkg.name.cyan(),
        pkg.version.yellow(),
        manager.cyan()
    );

    Ok(())
}

/// Fill in a missing `download_url` from the manifest's declared source
fn resolve_download_url(ctx: &AppContext, pkg: &mut Package) -> Result<(), BallError> {
    let quiet = ctx.flags.is_quiet();

    let resolved = match &pkg.source {
        PackageSource::GitHub { owner, repo } => {
            if owner.is_empty() || repo.is_empty() {
                return Err(BallError::InvalidConfig(format!(
                    "'{}' has no download_url and an incomplete github source",
                    pkg.name
                )));
            }

            info(
                quiet,
                format!(
                    "{} latest release of {}...",
                    "Scouting".green(),
                    format!("{}/{}", owner, repo).cyan()
                ),
            );
            GitHubRegistry::new(ctx.http_client.clone(), None)
                .fetch_package(&format!("{}/{}", owner, repo))?
        }
        PackageSource::Chocolatey { feed_url } => {
            info(
                quiet,
                format!(
                    "{} {} on Chocolatey...",
                    "Scouting".green(),
                    pkg.name.cyan()
                ),
            );
            let registry =
                ChocolateyRegistry::with_feed_url(ctx.http_client.clone(), feed_url.clone());
            registry.fetch_package(&pkg.name)?
        }
        PackageSource::BallerRegistry { url } => {
            info(
                quiet,
                format!(
                    "{} {} in the Baller registry...",
                    "Scouting".green(),
                    pkg.name.cyan()
                ),
            );
            BallerRegistryApi::new(ctx.http_client.clone(), url.clone()).fetch_package(&pkg.name)?
        }
        PackageSource::System { .. } => return Ok(()),
    };

    let download_url = resolved.download_url.ok_or_else(|| {
        BallError::NetworkError(format!(
            "no download URL found for '{}' — add 'download_url' to the manifest",
            pkg.name
        ))
    })?;

    if !resolved.version.is_empty() && resolved.version != pkg.version {
        info(
            quiet,
            format!(
                "{} {} -> {}",
                "Version".green(),
                pkg.version.yellow(),
                resolved.version.yellow()
            ),
        );
        pkg.version = resolved.version;
    }

    pkg.download_url = Some(download_url);

    if pkg.sha256.is_none() {
        pkg.sha256 = resolved.sha256;
        if pkg.hash_algorithm.is_none() {
            pkg.hash_algorithm = resolved.hash_algorithm;
        }
    }

    Ok(())
}

/// Manifest dependencies are recorded in the DB but not drafted by `build`
fn announce_dependencies(ctx: &AppContext, pkg: &Package) {
    if let Some(deps) = &pkg.dependencies {
        if !deps.is_empty() {
            info(
                ctx.flags.is_quiet(),
                format!(
                    "{} {} declared dependencies recorded — draft them separately",
                    "Bench".yellow(),
                    deps.len().to_string().cyan()
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(tag: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("baller_test_build_{}_{}", tag, nanos));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_resolve_manifest_path_prefers_toml_in_dir() {
        let dir = test_dir("toml_first");
        std::fs::write(dir.join("baller.toml"), "name = \"a\"\nversion = \"1.0.0\"").unwrap();
        std::fs::write(dir.join("baller.json"), "{}").unwrap();

        let resolved = resolve_manifest_path(dir.to_str().unwrap()).unwrap();
        assert_eq!(resolved, dir.join("baller.toml"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_manifest_path_falls_back_to_json_in_dir() {
        let dir = test_dir("json_fallback");
        std::fs::write(dir.join("baller.json"), "{}").unwrap();

        let resolved = resolve_manifest_path(dir.to_str().unwrap()).unwrap();
        assert_eq!(resolved, dir.join("baller.json"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_manifest_path_dir_without_manifest() {
        let dir = test_dir("empty_dir");

        let result = resolve_manifest_path(dir.to_str().unwrap());
        assert!(result.is_err());
        match result.unwrap_err() {
            BallError::InvalidConfig(msg) => assert!(msg.contains("no baller.toml or baller.json")),
            other => panic!("expected InvalidConfig, got {:?}", other),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_manifest_path_accepts_file() {
        let dir = test_dir("direct_file");
        let manifest = dir.join("custom.toml");
        std::fs::write(&manifest, "name = \"a\"\nversion = \"1.0.0\"").unwrap();

        let resolved = resolve_manifest_path(manifest.to_str().unwrap()).unwrap();
        assert_eq!(resolved, manifest);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_manifest_path_missing() {
        let result = resolve_manifest_path("/nonexistent/baller.toml");
        assert!(result.is_err());
        match result.unwrap_err() {
            BallError::InvalidConfig(msg) => assert!(msg.contains("manifest not found")),
            other => panic!("expected InvalidConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_no_deps_clears_manifest_dependencies() {
        let mut pkg = Package::new("pkg", "1.0.0");
        pkg.dependencies = Some(vec!["dep".to_string()]);

        let opts = BuildOptions {
            dry_run: false,
            no_deps: true,
            install_dir: None,
            force: false,
            source: None,
        };

        if opts.no_deps {
            pkg.dependencies = None;
        }

        assert!(pkg.dependencies.is_none());
    }
}
