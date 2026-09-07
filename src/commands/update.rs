use colored::Colorize;
use serde_json::json;

use crate::commands::draft::source_label;
use crate::context::AppContext;
use crate::core::db::InstalledPackage;
use crate::core::dep_solver::{get_installed_map, parse_version_flexible, resolve_deps};
use crate::core::hooks::{run_hook, HookType};
use crate::core::package::Package;
use crate::error::error::BallError;
use crate::platform::common::PlatformManager;
use crate::utils::output::{debug, info, print_json};

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

pub struct UpdateOptions {
    pub packages: Vec<String>,
    pub check: bool,
    pub include_frozen: bool,
}

pub fn execute_update(ctx: &AppContext, opts: &UpdateOptions) -> Result<(), BallError> {
    let quiet = ctx.flags.is_quiet();
    let pkgs = select_packages(ctx, opts)?;

    debug(format!("{} package(s) selected for update", pkgs.len()));

    let mut updated = 0u32;
    let mut failed = 0u32;
    let mut updated_names: Vec<String> = Vec::new();
    let mut stale: Vec<serde_json::Value> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut failures: Vec<serde_json::Value> = Vec::new();

    for pkg in &pkgs {
        if pkg.frozen && !opts.include_frozen {
            info(
                quiet,
                format!(
                    "{} {} is frozen, skipping",
                    "Frozen".cyan(),
                    pkg.name.cyan()
                ),
            );
            continue;
        }

        match ctx.registry.fetch_package(&pkg.name) {
            Ok(remote_pkg) => {
                // U1: Use semver-aware version comparison with fallback to string
                let current_version = parse_version_flexible(&pkg.version);
                let remote = parse_version_flexible(&remote_pkg.version);
                let needs_update = match (current_version, remote) {
                    (Some(cur), Some(rem)) => rem > cur,
                    _ => remote_pkg.version != pkg.version, // fallback to string comparison
                };

                debug(format!(
                    "{}: installed v{}, registry v{} from {} -> {}",
                    pkg.name,
                    pkg.version,
                    remote_pkg.version,
                    source_label(&remote_pkg.source),
                    if needs_update { "stale" } else { "current" }
                ));
                debug(format!(
                    "{}: registry asset url {}",
                    pkg.name,
                    remote_pkg
                        .download_url
                        .as_deref()
                        .unwrap_or("<resolved by source>")
                ));

                if !needs_update {
                    info(
                        quiet,
                        format!("{} {} is up-to-date", "OK".green(), pkg.name.cyan()),
                    );
                    current.push(pkg.name.clone());
                    continue;
                }

                if opts.check {
                    info(
                        quiet,
                        format!(
                            "{} {}: {} -> {}",
                            "Stale".yellow(),
                            pkg.name.cyan(),
                            pkg.version.yellow(),
                            remote_pkg.version.green()
                        ),
                    );
                    stale.push(json!({
                        "name": pkg.name,
                        "installed": pkg.version,
                        "available": remote_pkg.version,
                        "frozen": pkg.frozen,
                    }));
                    continue;
                }

                info(
                    quiet,
                    format!(
                        "{} {}: {} -> {}",
                        "Updating".green(),
                        pkg.name.cyan(),
                        pkg.version.yellow(),
                        remote_pkg.version.green()
                    ),
                );

                // U2: Resolve dependencies for the updated package
                let installed = get_installed_map(&ctx.db);
                let resolve_result = resolve_deps(&remote_pkg.name, &ctx.registry, &installed)?;

                let missing_deps: Vec<&Package> = resolve_result
                    .packages
                    .iter()
                    .filter(|dep| dep.name != pkg.name && !installed.contains_key(&dep.name))
                    .collect();

                // Install any missing dependencies first
                for dep in &missing_deps {
                    info(
                        quiet,
                        format!(
                            "  {} new dependency: {}",
                            "Fetching".cyan(),
                            dep.name.cyan()
                        ),
                    );
                    debug(format!(
                        "{}: fetching dependency {} v{} from {}",
                        pkg.name,
                        dep.name,
                        dep.version,
                        dep.download_url
                            .as_deref()
                            .unwrap_or("<resolved by source>")
                    ));

                    let downloaded = ctx.downloader.download_and_extract(dep, !quiet)?;

                    debug(format!(
                        "{}: dependency extracted to {}",
                        dep.name,
                        downloaded.extract_dir.display()
                    ));

                    let install_path = downloaded.extract_dir.to_string_lossy().to_string();
                    let bin_path_str = downloaded
                        .binary_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string());

                    if let Some(binary_path) = &downloaded.binary_path {
                        ActiveManager::create_symlink(binary_path, &dep.name)?;
                    }

                    ctx.db.insert_package(
                        dep,
                        &install_path,
                        bin_path_str.as_deref(),
                        None,
                        false,
                    )?;
                }

                // U4: Pre-update hook receives both old and new versions
                let pre_env = [
                    ("BALLER_NEW_VERSION", remote_pkg.version.as_str()),
                    ("BALLER_OLD_VERSION", pkg.version.as_str()),
                ];
                run_hook(
                    &HookType::PreUpdate,
                    &pkg.name,
                    &remote_pkg.version,
                    &ctx.config.hooks_dir,
                    &ctx.config.hooks,
                    &pre_env,
                )?;

                debug(format!(
                    "{}: fetching v{} into cache {}",
                    pkg.name,
                    remote_pkg.version,
                    ctx.config.cache_dir.display()
                ));

                let downloaded = ctx.downloader.download_and_extract(&remote_pkg, !quiet)?;

                debug(format!(
                    "{}: extracted to {}",
                    remote_pkg.name,
                    downloaded.extract_dir.display()
                ));

                // U3: Clean old extracted directory before installing new one
                let old_extract_dir = ctx
                    .config
                    .cache_dir
                    .join(format!("{}-{}", pkg.name, pkg.version));
                debug(format!(
                    "{}: pruning stale extract dir {}",
                    pkg.name,
                    old_extract_dir.display()
                ));
                let _ = std::fs::remove_dir_all(&old_extract_dir);

                if let Some(binary_path) = &downloaded.binary_path {
                    ActiveManager::create_symlink(binary_path, &remote_pkg.name)?;
                }

                let install_path = downloaded.extract_dir.to_string_lossy().to_string();
                let bin_path_str = downloaded
                    .binary_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string());

                ctx.db.insert_package(
                    &remote_pkg,
                    &install_path,
                    bin_path_str.as_deref(),
                    None,
                    true,
                )?;

                // U4: Post-update hook receives both old and new versions
                let post_env = [
                    ("BALLER_NEW_VERSION", remote_pkg.version.as_str()),
                    ("BALLER_OLD_VERSION", pkg.version.as_str()),
                ];
                run_hook(
                    &HookType::PostUpdate,
                    &remote_pkg.name,
                    &remote_pkg.version,
                    &ctx.config.hooks_dir,
                    &ctx.config.hooks,
                    &post_env,
                )?;

                updated += 1;
                updated_names.push(pkg.name.clone());
            }
            Err(e) => {
                info(
                    quiet,
                    format!(
                        "{} {} update failed: {}",
                        "Failed".red(),
                        pkg.name.cyan(),
                        e
                    ),
                );
                failed += 1;
                failures.push(json!({ "name": pkg.name, "error": e.to_string() }));
            }
        }
    }

    if ctx.flags.json {
        return print_json(&json!({
            "command": "update",
            "checked": pkgs.len(),
            "dry_run": opts.check,
            "updated": updated_names,
            "stale": stale,
            "up_to_date": current,
            "failed": failures,
        }));
    }

    if opts.check {
        if stale.is_empty() {
            println!("{} All packages are up-to-date", "OK".green());
        } else {
            println!(
                "\n{} {} package(s) can be updated — run {} to apply",
                "Done".green().bold(),
                stale.len(),
                "baller update".cyan()
            );
        }
        return Ok(());
    }

    if updated > 0 || failed > 0 {
        println!(
            "\n{} Update complete: {} updated, {} failed",
            "Done".green().bold(),
            updated,
            failed
        );
    } else {
        println!("{} All packages are up-to-date", "OK".green());
    }

    Ok(())
}

/// Everything installed, or just the packages named on the command line
fn select_packages(
    ctx: &AppContext,
    opts: &UpdateOptions,
) -> Result<Vec<InstalledPackage>, BallError> {
    let installed = ctx.db.list_packages()?;

    if opts.packages.is_empty() {
        return Ok(installed);
    }

    let mut selected = Vec::new();
    for name in &opts.packages {
        match installed.iter().find(|pkg| &pkg.name == name) {
            Some(pkg) => selected.push(pkg.clone()),
            None => return Err(BallError::PackageNotFound(name.clone())),
        }
    }

    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_options_default_to_full_run() {
        let opts = UpdateOptions {
            packages: Vec::new(),
            check: false,
            include_frozen: false,
        };
        assert!(opts.packages.is_empty());
        assert!(!opts.check);
        assert!(!opts.include_frozen);
    }
}
