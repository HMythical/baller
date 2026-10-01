use colored::Colorize;
use serde_json::json;

use crate::commands::draft::{screen_artifact, source_label};
use crate::context::AppContext;
use crate::core::db::InstalledPackage;
use crate::core::dep_solver::{get_installed_map, parse_version_flexible, resolve_deps};
use crate::core::hooks::{run_hook, HookType};
use crate::core::package::{Package, PackageSource};
use crate::core::registry::RegistrySource;
use crate::error::error::BallError;
use crate::platform::common::PlatformManager;
use crate::security::ranges::parse_advisory_version;
use crate::utils::output::print_json;

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

    tracing::debug!("{} package(s) selected for update", pkgs.len());

    let mut updated = 0u32;
    let mut failed = 0u32;
    let mut updated_names: Vec<String> = Vec::new();
    let mut stale: Vec<serde_json::Value> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut failures: Vec<serde_json::Value> = Vec::new();

    for pkg in &pkgs {
        if pkg.frozen && !opts.include_frozen {
            tracing::info!(
                "{} {} is frozen, skipping",
                "Frozen".cyan(),
                pkg.name.cyan()
            );
            continue;
        }

        match ctx.registry.fetch_package(&pkg.name) {
            Ok(remote_pkg) => {
                // U1: Use semver-aware version comparison with fallback to string
                let needs_update = is_newer_release(
                    &pkg.version,
                    &remote_pkg.version,
                    is_distro_versioned(&pkg.source, &remote_pkg.source),
                );

                tracing::debug!(
                    "{}: installed v{}, registry v{} from {} -> {}",
                    pkg.name,
                    pkg.version,
                    remote_pkg.version,
                    source_label(&remote_pkg.source),
                    if needs_update { "stale" } else { "current" }
                );
                tracing::debug!(
                    "{}: registry asset url {}",
                    pkg.name,
                    remote_pkg
                        .download_url
                        .as_deref()
                        .unwrap_or("<resolved by source>")
                );

                if !needs_update {
                    tracing::info!("{} {} is up-to-date", "OK".green(), pkg.name.cyan(),);
                    current.push(pkg.name.clone());
                    continue;
                }

                if opts.check {
                    tracing::info!(
                        "{} {}: {} -> {}",
                        "Stale".yellow(),
                        pkg.name.cyan(),
                        pkg.version.yellow(),
                        remote_pkg.version.green()
                    );
                    stale.push(json!({
                        "name": pkg.name,
                        "installed": pkg.version,
                        "available": remote_pkg.version,
                        "frozen": pkg.frozen,
                    }));
                    continue;
                }

                tracing::info!(
                    "{} {}: {} -> {}",
                    "Updating".green(),
                    pkg.name.cyan(),
                    pkg.version.yellow(),
                    remote_pkg.version.green()
                );

                // U2: Resolve dependencies for the updated package
                let installed = get_installed_map(&ctx.db);
                let resolve_result = resolve_deps(&remote_pkg.name, &ctx.registry, &installed)?;

                // `update` stays best-effort: it re-derives `missing_deps` from
                // the subset below, but an unresolvable dependency must at least
                // be visible, never silent (#73).
                for name in &resolve_result.unresolved {
                    tracing::warn!(
                        "dependency '{}' of '{}' could not be resolved; skipping it",
                        name,
                        pkg.name
                    );
                }

                let missing_deps: Vec<&Package> = resolve_result
                    .packages
                    .iter()
                    .filter(|dep| dep.name != pkg.name && !installed.contains_key(&dep.name))
                    .collect();

                // Referee Phase A: the upgrade and every dependency it pulls in
                // are checked before anything is downloaded, so a blocked
                // upgrade leaves the working installed version untouched.
                let mut plan: Vec<Package> = vec![remote_pkg.clone()];
                plan.extend(missing_deps.iter().map(|dep| (*dep).clone()));

                let gate = ctx.referee.gate(&ctx.db, &plan)?;
                gate.report(quiet);
                if let Some(blocked) = gate.block_error() {
                    return Err(blocked);
                }

                // Install any missing dependencies first
                for dep in &missing_deps {
                    tracing::info!(
                        "  {} new dependency: {}",
                        "Fetching".cyan(),
                        dep.name.cyan()
                    );
                    tracing::debug!(
                        "{}: fetching dependency {} v{} from {}",
                        pkg.name,
                        dep.name,
                        dep.version,
                        dep.download_url
                            .as_deref()
                            .unwrap_or("<resolved by source>")
                    );

                    let downloaded = ctx.downloader.download_and_extract(dep, !quiet)?;

                    tracing::debug!(
                        "{}: dependency extracted to {}",
                        dep.name,
                        downloaded.extract_dir.display()
                    );

                    screen_artifact(ctx, dep, &downloaded)?;

                    let install_path = downloaded.extract_dir.to_string_lossy().to_string();

                    // A dependency that extracts without an executable is not
                    // installed, so it must not reach the database either.
                    let binary_path = match downloaded.binary_path.as_ref() {
                        Some(path) => path.clone(),
                        None => return Err(ctx.downloader.no_binary_error(dep, &downloaded)),
                    };
                    let bin_path_str = Some(binary_path.to_string_lossy().to_string());

                    ActiveManager::create_symlink(&binary_path, &dep.name)?;

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

                tracing::debug!(
                    "{}: fetching v{} into cache {}",
                    pkg.name,
                    remote_pkg.version,
                    ctx.config.cache_dir.display()
                );

                let downloaded = ctx.downloader.download_and_extract(&remote_pkg, !quiet)?;

                tracing::debug!(
                    "{}: extracted to {}",
                    remote_pkg.name,
                    downloaded.extract_dir.display()
                );

                // Referee Phase B runs *before* the old extract directory is
                // pruned: a rejected upgrade must not also cost the user the
                // version they already had working.
                screen_artifact(ctx, &remote_pkg, &downloaded)?;

                // U3: Clean old extracted directory before installing new one
                let old_extract_dir = ctx
                    .config
                    .cache_dir
                    .join(format!("{}-{}", pkg.name, pkg.version));
                tracing::debug!(
                    "{}: pruning stale extract dir {}",
                    pkg.name,
                    old_extract_dir.display()
                );
                let _ = std::fs::remove_dir_all(&old_extract_dir);

                // The old extract dir is already gone; if the new archive holds
                // no executable there is nothing to link, so fail instead of
                // recording a version bump the user cannot run.
                let binary_path = match downloaded.binary_path.as_ref() {
                    Some(path) => path.clone(),
                    None => return Err(ctx.downloader.no_binary_error(&remote_pkg, &downloaded)),
                };

                ActiveManager::create_symlink(&binary_path, &remote_pkg.name)?;

                let install_path = downloaded.extract_dir.to_string_lossy().to_string();
                let bin_path_str = Some(binary_path.to_string_lossy().to_string());

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
                tracing::info!(
                    "{} {} update failed: {}",
                    "Failed".red(),
                    pkg.name.cyan(),
                    e
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

/// Whether the registry's version is newer than the installed one.
///
/// Upstream releases are compared with [`parse_advisory_version`], so an
/// installed `1.0.0-rc1` sees `1.0.0` as the newer release — the lossy parser
/// collapses both to 1.0.0 and the rc was never offered its final release.
/// Distribution versions keep [`parse_version_flexible`]: a Debian
/// `1.2.3-5ubuntu10` also parses as a semver pre-release, and semver compares
/// `5ubuntu10` below `5ubuntu2` lexically, which would offer a downgrade as an
/// update. Anything unparseable falls back to a plain string comparison.
pub(crate) fn is_newer_release(installed: &str, remote: &str, distro: bool) -> bool {
    let parse = if distro {
        parse_version_flexible
    } else {
        parse_advisory_version
    };
    match (parse(installed), parse(remote)) {
        (Some(cur), Some(rem)) => rem > cur,
        _ => remote != installed,
    }
}

/// Whether either side of a comparison is a system package manager's version.
pub(crate) fn is_distro_versioned(installed_source: &str, remote_source: &PackageSource) -> bool {
    installed_source == RegistrySource::System.db_name()
        || matches!(remote_source, PackageSource::System { .. })
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

    #[test]
    fn test_final_release_is_newer_than_its_rc() {
        assert!(is_newer_release("1.0.0-rc1", "1.0.0", false));
        assert!(is_newer_release("1.0.0-rc1", "1.0.0-rc2", false));
        assert!(!is_newer_release("1.0.0", "1.0.0-rc1", false));
        assert!(!is_newer_release("1.0.0", "1.0.0", false));
    }

    #[test]
    fn test_distro_versions_keep_the_lossy_comparison() {
        // Lexically `5ubuntu2` > `5ubuntu10`; the lossy parser sees no change
        // rather than offering a downgrade.
        assert!(!is_newer_release("1.2.3-5ubuntu10", "1.2.3-5ubuntu2", true));
        assert!(is_newer_release("1.2.3-5ubuntu10", "1.2.4-1", true));
        assert!(is_distro_versioned("system", &PackageSource::default()));
        assert!(!is_distro_versioned(
            "github",
            &PackageSource::Cargo {
                crate_name: String::new()
            }
        ));
    }

    #[test]
    fn test_unparseable_versions_compare_as_strings() {
        assert!(is_newer_release("nightly", "nightly-2", false));
        assert!(!is_newer_release("nightly", "nightly", false));
    }
}
