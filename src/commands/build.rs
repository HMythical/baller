use colored::Colorize;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::commands::draft::source_label;
use crate::context::AppContext;
use crate::core::hooks::{run_hook, HookType};
use crate::core::manifest::{parse_github_url, ManifestParser};
use crate::core::package::{Package, PackageSource};
use crate::core::registry::RegistrySource;
use crate::error::error::BallError;
use crate::http::cargo::install_cargo_package;
use crate::http::chocolatey::ChocolateyRegistry;
use crate::http::github::GitHubRegistry;
use crate::http::registry_api::BallerRegistryApi;
use crate::http::system::install_system_package;
use crate::platform::common::PlatformManager;
use crate::utils::output::print_json;

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

const MANIFEST_NAMES: [&str; 2] = ["baller.toml", "baller.json"];
const DEFAULT_CARGO_VERSION: &str = "0.0.0";
const CARGO_ERROR_LINES: usize = 12;

pub struct BuildOptions {
    pub dry_run: bool,
    pub no_deps: bool,
    pub install_dir: Option<String>,
    pub force: bool,
    pub source: Option<RegistrySource>,
}

pub fn execute_build(ctx: &AppContext, path: &str, opts: &BuildOptions) -> Result<(), BallError> {
    let quiet = ctx.flags.is_quiet();
    let manifest_path = match resolve_manifest_path(path) {
        Ok(manifest) => manifest,
        Err(err) => match detect_cargo_project(path) {
            Some(project_dir) => return build_cargo_project(ctx, &project_dir, opts),
            None => return Err(err),
        },
    };
    let manifest_str = manifest_path.to_string_lossy().to_string();

    tracing::info!("{} {}...", "Building".green().bold(), manifest_str.cyan(),);

    tracing::debug!("manifest: {}", manifest_path.display());

    let mut pkg = ManifestParser::parse_auto(&manifest_path)?;
    ManifestParser::validate(&pkg)?;

    if let Some(source) = &opts.source {
        tracing::debug!(
            "overriding manifest source with --source {}",
            source.config_name()
        );
        override_source(ctx, &mut pkg, source)?;
    }

    tracing::debug!(
        "parsed {} v{}, effective source {}",
        pkg.name,
        pkg.version,
        source_label(&pkg.source)
    );

    if opts.no_deps {
        tracing::debug!("manifest dependencies dropped (--no-deps)");
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

    if let PackageSource::Cargo { crate_name } = &pkg.source {
        return build_cargo_package(ctx, &pkg, crate_name, &manifest_str);
    }

    if pkg.download_url.is_none() {
        resolve_download_url(ctx, &mut pkg)?;
    }

    tracing::debug!(
        "fetching {} into cache {}",
        pkg.download_url
            .as_deref()
            .unwrap_or("<resolved by source>"),
        ctx.config.cache_dir.display()
    );

    let downloaded = ctx.downloader.download_and_extract(&pkg, !quiet)?;

    tracing::debug!(
        "extracted to {} (binary: {})",
        downloaded.extract_dir.display(),
        downloaded
            .binary_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none found".to_string())
    );

    let install_path = downloaded.extract_dir.to_string_lossy().to_string();
    let bin_path_str = downloaded
        .binary_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    if let Some(binary_path) = &downloaded.binary_path {
        match &opts.install_dir {
            Some(dir) => {
                ActiveManager::create_symlink_in(Path::new(dir), binary_path, &pkg.name)?;
                tracing::info!("{} linked into {}", "Linked".green(), dir.cyan(),);
            }
            None => {
                ActiveManager::create_symlink(binary_path, &pkg.name)?;
                tracing::info!(
                    "{} symlinked to {}",
                    "Linked".green(),
                    binary_path.display().to_string().cyan(),
                );
            }
        }
    } else {
        tracing::info!(
            "{} no binary found in extracted package",
            "Warning".yellow()
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

    announce_dependencies(&pkg);

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
        RegistrySource::Cargo => {
            ctx.registry.cargo_manager_name().ok_or_else(|| {
                BallError::PackageManagerError(
                    "--source cargo needs a cargo toolchain on PATH".to_string(),
                )
            })?;
            PackageSource::Cargo {
                crate_name: pkg.name.clone(),
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

    tracing::info!("{} installing via {}...", "System".green(), manager.cyan(),);
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

    announce_dependencies(pkg);

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

/// Install a manifest that names a crate via `cargo install`
fn build_cargo_package(
    ctx: &AppContext,
    pkg: &Package,
    crate_name: &str,
    manifest_path: &str,
) -> Result<(), BallError> {
    tracing::info!(
        "{} installing {} via cargo...",
        "Cargo".green(),
        crate_name.cyan()
    );
    install_cargo_package(crate_name)?;

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

    announce_dependencies(pkg);

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
        "cargo".cyan()
    );

    Ok(())
}

/// A build target that is a directory holding a Cargo project
fn detect_cargo_project(path: &str) -> Option<PathBuf> {
    let candidate = Path::new(path);

    if candidate.is_dir() && candidate.join("Cargo.toml").is_file() {
        return Some(candidate.to_path_buf());
    }

    None
}

#[derive(Debug)]
struct CargoMeta {
    name: String,
    version: String,
    bin: String,
}

/// Read the crate name, version and binary name out of `<dir>/Cargo.toml`
fn read_cargo_meta(dir: &Path) -> Result<CargoMeta, BallError> {
    let manifest = dir.join("Cargo.toml");
    let content = std::fs::read_to_string(&manifest).map_err(|e| {
        BallError::InvalidConfig(format!("failed to read {}: {}", manifest.display(), e))
    })?;

    let raw: toml::Value = toml::from_str(&content).map_err(|e| {
        BallError::InvalidConfig(format!("Failed to parse {}: {}", manifest.display(), e))
    })?;

    let package = raw.get("package");

    let name = package
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .ok_or_else(|| {
            BallError::InvalidConfig(format!(
                "{} has no [package] name — workspace roots are not supported",
                manifest.display()
            ))
        })?
        .to_string();

    let version = package
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_CARGO_VERSION)
        .to_string();

    let bin = raw
        .get("bin")
        .and_then(|b| b.as_array())
        .and_then(|bins| bins.first())
        .and_then(|first| first.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or(&name)
        .to_string();

    Ok(CargoMeta { name, version, bin })
}

/// The file names cargo may have written for `bin_name`, in preference order
fn cargo_binary_names(bin_name: &str) -> Vec<String> {
    let mut names = Vec::new();

    for base in [bin_name.to_string(), bin_name.replace('-', "_")] {
        let candidate = if cfg!(target_os = "windows") {
            format!("{}.exe", base)
        } else {
            base
        };

        if !names.contains(&candidate) {
            names.push(candidate);
        }
    }

    names
}

/// Release-directory entries that could be the program itself
fn is_executable_artifact(path: &Path) -> bool {
    let extension = path.extension().and_then(|ext| ext.to_str());

    if cfg!(target_os = "windows") {
        extension == Some("exe")
    } else {
        extension.is_none()
    }
}

/// Find the artifact cargo built for `bin_name` inside a `target/release` directory
fn locate_cargo_binary(release_dir: &Path, bin_name: &str) -> Option<PathBuf> {
    for name in cargo_binary_names(bin_name) {
        let candidate = release_dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let wanted = bin_name.replace('-', "_").to_lowercase();

    for entry in std::fs::read_dir(release_dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_executable_artifact(&path) {
            continue;
        }

        let stem = match path.file_stem() {
            Some(stem) => stem.to_string_lossy().replace('-', "_").to_lowercase(),
            None => continue,
        };

        if stem == wanted {
            return Some(path);
        }
    }

    None
}

/// Trim cargo's stderr down to the tail that explains the failure
fn cargo_error_snippet(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();

    if lines.is_empty() {
        return "no output".to_string();
    }

    lines[lines.len().saturating_sub(CARGO_ERROR_LINES)..].join("\n")
}

/// Compile a crate in release mode
fn compile_cargo_project(dir: &Path) -> Result<(), BallError> {
    let output = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(dir)
        .output()
        .map_err(|e| BallError::PackageManagerError(format!("failed to run cargo: {}", e)))?;

    if !output.status.success() {
        return Err(BallError::PackageManagerError(format!(
            "cargo build --release failed in '{}':\n{}",
            dir.display(),
            cargo_error_snippet(&String::from_utf8_lossy(&output.stderr))
        )));
    }

    Ok(())
}

/// The bin directory a cargo project is linked into
fn cargo_install_dir(opts: &BuildOptions) -> String {
    match &opts.install_dir {
        Some(dir) => dir.clone(),
        None => ActiveManager::get_install_dir()
            .map(|dir| dir.to_string_lossy().to_string())
            .unwrap_or_else(|_| "the platform default".to_string()),
    }
}

/// `--dry-run` for a Cargo project: report the compile plan, touching nothing
fn report_cargo_plan(
    ctx: &AppContext,
    pkg: &Package,
    meta: &CargoMeta,
    dir: &Path,
    release_dir: &Path,
    opts: &BuildOptions,
) -> Result<(), BallError> {
    let already_installed = ctx.db.package_exists(&pkg.name).unwrap_or(false);
    let project_dir = dir.to_string_lossy().to_string();
    let install_dir = cargo_install_dir(opts);
    let expected_binary = locate_cargo_binary(release_dir, &meta.bin)
        .unwrap_or_else(|| release_dir.join(&cargo_binary_names(&meta.bin)[0]))
        .to_string_lossy()
        .to_string();

    if ctx.flags.json {
        return print_json(&json!({
            "command": "build",
            "project": project_dir,
            "package": pkg.name,
            "version": pkg.version,
            "source": source_label(&pkg.source),
            "bin_path": expected_binary,
            "install_dir": install_dir,
            "already_installed": already_installed,
            "dry_run": true,
        }));
    }

    println!(
        "{} {} v{} from {}",
        "Dry run".yellow().bold(),
        pkg.name.cyan(),
        pkg.version.yellow(),
        "cargo build --release".cyan()
    );
    println!("  {} {}", "Project:".yellow(), project_dir);
    println!("  {} {}", "Binary:".yellow(), expected_binary);
    println!("  {} {}", "Install dir:".yellow(), install_dir);
    println!(
        "  {} resolved by cargo while compiling",
        "Dependencies:".yellow()
    );
    if already_installed {
        println!(
            "  {} already on the roster — needs {}",
            "Note:".yellow(),
            "--force".cyan()
        );
    }

    println!("{} nothing was compiled", "Note".yellow());
    Ok(())
}

/// Compile a Cargo project from source and install the binary it produces
fn build_cargo_project(
    ctx: &AppContext,
    dir: &Path,
    opts: &BuildOptions,
) -> Result<(), BallError> {
    if opts.source.is_some() {
        return Err(BallError::InvalidConfig(
            "--source does not apply to a Cargo project — its sources are compiled from disk"
                .to_string(),
        ));
    }

    if opts.no_deps {
        return Err(BallError::InvalidConfig(
            "--no-deps does not apply to a Cargo project — cargo resolves its own dependencies"
                .to_string(),
        ));
    }

    let project_dir = dir.to_string_lossy().to_string();
    let manifest_str = dir.join("Cargo.toml").to_string_lossy().to_string();

    tracing::info!("{} {}...", "Building".green().bold(), manifest_str.cyan(),);

    let meta = read_cargo_meta(dir)?;
    let release_dir = dir.join("target").join("release");

    let mut pkg = Package::new(&meta.name, &meta.version);
    pkg.source = PackageSource::Cargo {
        crate_name: meta.name.clone(),
    };

    if opts.dry_run {
        return report_cargo_plan(ctx, &pkg, &meta, dir, &release_dir, opts);
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

    tracing::info!("{} cargo build --release...", "Compiling".green(),);
    compile_cargo_project(dir)?;

    let binary_path = locate_cargo_binary(&release_dir, &meta.bin).ok_or_else(|| {
        BallError::PackageManagerError(format!(
            "cargo build --release left no '{}' binary in {}",
            meta.bin,
            release_dir.display()
        ))
    })?;

    match &opts.install_dir {
        Some(install_dir) => {
            ActiveManager::create_symlink_in(Path::new(install_dir), &binary_path, &pkg.name)?;
            tracing::info!("{} linked into {}", "Linked".green(), install_dir.cyan(),);
        }
        None => {
            ActiveManager::create_symlink(&binary_path, &pkg.name)?;
            tracing::info!(
                "{} symlinked to {}",
                "Linked".green(),
                binary_path.display().to_string().cyan(),
            );
        }
    }

    let bin_path_str = binary_path.to_string_lossy().to_string();

    ctx.db
        .insert_package(&pkg, "", Some(&bin_path_str), Some(&manifest_str), true)?;

    let post_env = [
        ("BALLER_INSTALL_PATH", project_dir.as_str()),
        ("BALLER_BIN_PATH", bin_path_str.as_str()),
    ];
    run_hook(
        &HookType::PostInstall,
        &pkg.name,
        &pkg.version,
        &ctx.config.hooks_dir,
        &ctx.config.hooks,
        &post_env,
    )?;

    if ctx.flags.json {
        return print_json(&json!({
            "command": "build",
            "project": project_dir,
            "package": pkg.name,
            "version": pkg.version,
            "source": source_label(&pkg.source),
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

/// Fill in a missing `download_url` from the manifest's declared source
fn resolve_download_url(ctx: &AppContext, pkg: &mut Package) -> Result<(), BallError> {
    let resolved = match &pkg.source {
        PackageSource::GitHub { owner, repo } => {
            if owner.is_empty() || repo.is_empty() {
                return Err(BallError::InvalidConfig(format!(
                    "'{}' has no download_url and an incomplete github source",
                    pkg.name
                )));
            }

            tracing::info!(
                "{} latest release of {}...",
                "Scouting".green(),
                format!("{}/{}", owner, repo).cyan()
            );
            GitHubRegistry::new(ctx.http_client.clone(), None)
                .fetch_package(&format!("{}/{}", owner, repo))?
        }
        PackageSource::Chocolatey { feed_url } => {
            tracing::info!(
                "{} {} on Chocolatey...",
                "Scouting".green(),
                pkg.name.cyan()
            );
            let registry =
                ChocolateyRegistry::with_feed_url(ctx.http_client.clone(), feed_url.clone());
            registry.fetch_package(&pkg.name)?
        }
        PackageSource::BallerRegistry { url } => {
            tracing::info!(
                "{} {} in the Baller registry...",
                "Scouting".green(),
                pkg.name.cyan()
            );
            BallerRegistryApi::new(ctx.http_client.clone(), url.clone()).fetch_package(&pkg.name)?
        }
        PackageSource::System { .. } | PackageSource::Cargo { .. } => return Ok(()),
    };

    let download_url = resolved.download_url.ok_or_else(|| {
        BallError::NetworkError(format!(
            "no download URL found for '{}' — add 'download_url' to the manifest",
            pkg.name
        ))
    })?;

    if !resolved.version.is_empty() && resolved.version != pkg.version {
        tracing::info!(
            "{} {} -> {}",
            "Version".green(),
            pkg.version.yellow(),
            resolved.version.yellow()
        );
        pkg.version = resolved.version;
    }

    tracing::debug!("resolved download url for {}: {}", pkg.name, download_url);

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
fn announce_dependencies(pkg: &Package) {
    if let Some(deps) = &pkg.dependencies {
        if !deps.is_empty() {
            tracing::info!(
                "{} {} declared dependencies recorded — draft them separately",
                "Bench".yellow(),
                deps.len().to_string().cyan()
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

    #[test]
    fn test_detect_cargo_project_finds_manifest() {
        let dir = test_dir("cargo_detect");
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"tool\"").unwrap();

        assert_eq!(
            detect_cargo_project(dir.to_str().unwrap()),
            Some(dir.clone())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_detect_cargo_project_ignores_plain_dir_and_files() {
        let dir = test_dir("cargo_detect_none");
        let manifest = dir.join("Cargo.toml");
        std::fs::write(&manifest, "[package]\nname = \"tool\"").unwrap();

        let empty = test_dir("cargo_detect_empty");

        assert!(detect_cargo_project(empty.to_str().unwrap()).is_none());
        assert!(detect_cargo_project(manifest.to_str().unwrap()).is_none());
        assert!(detect_cargo_project("/nonexistent/project").is_none());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn test_read_cargo_meta_reads_name_version_and_bin() {
        let dir = test_dir("cargo_meta");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"my-tool\"\nversion = \"1.2.3\"\n\n[[bin]]\nname = \"mt\"\npath = \"src/main.rs\"\n",
        )
        .unwrap();

        let meta = read_cargo_meta(&dir).unwrap();
        assert_eq!(meta.name, "my-tool");
        assert_eq!(meta.version, "1.2.3");
        assert_eq!(meta.bin, "mt");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_cargo_meta_defaults_version_and_bin() {
        let dir = test_dir("cargo_meta_defaults");
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"my-tool\"\n").unwrap();

        let meta = read_cargo_meta(&dir).unwrap();
        assert_eq!(meta.version, DEFAULT_CARGO_VERSION);
        assert_eq!(meta.bin, "my-tool");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_cargo_meta_requires_a_package_name() {
        let dir = test_dir("cargo_meta_workspace");
        std::fs::write(dir.join("Cargo.toml"), "[workspace]\nmembers = [\"a\"]\n").unwrap();

        match read_cargo_meta(&dir).unwrap_err() {
            BallError::InvalidConfig(msg) => assert!(msg.contains("no [package] name")),
            other => panic!("expected InvalidConfig, got {:?}", other),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cargo_binary_names_dedupe_and_platform_suffix() {
        let hyphenated = cargo_binary_names("my-tool");
        assert_eq!(hyphenated.len(), 2);

        let plain = cargo_binary_names("tool");
        assert_eq!(plain.len(), 1);

        if cfg!(target_os = "windows") {
            assert_eq!(hyphenated, vec!["my-tool.exe", "my_tool.exe"]);
            assert_eq!(plain, vec!["tool.exe"]);
        } else {
            assert_eq!(hyphenated, vec!["my-tool", "my_tool"]);
            assert_eq!(plain, vec!["tool"]);
        }
    }

    #[test]
    fn test_locate_cargo_binary_prefers_the_exact_name() {
        let dir = test_dir("cargo_locate_exact");
        let names = cargo_binary_names("my-tool");
        std::fs::write(dir.join(&names[0]), "binary").unwrap();
        std::fs::write(dir.join(&names[1]), "binary").unwrap();

        assert_eq!(
            locate_cargo_binary(&dir, "my-tool"),
            Some(dir.join(&names[0]))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_locate_cargo_binary_falls_back_to_underscores() {
        let dir = test_dir("cargo_locate_underscore");
        let names = cargo_binary_names("my-tool");
        std::fs::write(dir.join(&names[1]), "binary").unwrap();

        assert_eq!(
            locate_cargo_binary(&dir, "my-tool"),
            Some(dir.join(&names[1]))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_locate_cargo_binary_scans_for_a_case_insensitive_match() {
        let dir = test_dir("cargo_locate_scan");
        let scanned = &cargo_binary_names("My-Tool")[1];
        std::fs::write(dir.join(scanned), "binary").unwrap();
        std::fs::write(dir.join("my_tool.d"), "deps").unwrap();

        assert_eq!(
            locate_cargo_binary(&dir, "my-tool"),
            Some(dir.join(scanned))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_locate_cargo_binary_missing_release_dir() {
        assert!(locate_cargo_binary(Path::new("/nonexistent/release"), "tool").is_none());
    }

    #[test]
    fn test_cargo_error_snippet_keeps_the_tail() {
        let stderr = (1..=20)
            .map(|n| format!("line {}", n))
            .collect::<Vec<_>>()
            .join("\n\n");

        let snippet = cargo_error_snippet(&stderr);
        let lines: Vec<&str> = snippet.lines().collect();
        assert_eq!(lines.len(), CARGO_ERROR_LINES);
        assert_eq!(lines[CARGO_ERROR_LINES - 1], "line 20");

        assert_eq!(cargo_error_snippet("   \n\n"), "no output");
    }
}
