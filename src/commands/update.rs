use colored::Colorize;

use crate::context::AppContext;
use crate::core::dep_solver::{get_installed_map, parse_version_flexible, resolve_deps};
use crate::core::hooks::{run_hook, HookType};
use crate::core::package::Package;
use crate::error::error::BallError;
use crate::platform::common::PlatformManager;

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

pub fn execute_update(ctx: &AppContext) -> Result<(), BallError> {
    let pkgs = ctx.db.list_packages()?;
    let mut updated = 0u32;
    let mut failed = 0u32;

    for pkg in &pkgs {
        if pkg.frozen {
            println!(
                "{} {} is frozen, skipping",
                "Frozen".cyan(),
                pkg.name.cyan()
            );
            continue;
        }

        match ctx.registry.fetch_package(&pkg.name) {
            Ok(remote_pkg) => {
                // U1: Use semver-aware version comparison with fallback to string
                let current = parse_version_flexible(&pkg.version);
                let remote = parse_version_flexible(&remote_pkg.version);
                let needs_update = match (current, remote) {
                    (Some(cur), Some(rem)) => rem > cur,
                    _ => remote_pkg.version != pkg.version, // fallback to string comparison
                };

                if needs_update {
                    println!(
                        "{} {}: {} -> {}",
                        "Updating".green(),
                        pkg.name.cyan(),
                        pkg.version.yellow(),
                        remote_pkg.version.green()
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
                        println!(
                            "  {} new dependency: {}",
                            "Fetching".cyan(),
                            dep.name.cyan()
                        );
                        let downloaded = ctx.downloader.download_and_extract(dep, true)?;

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

                    let downloaded = ctx.downloader.download_and_extract(&remote_pkg, true)?;

                    // U3: Clean old extracted directory before installing new one
                    let old_extract_dir = ctx
                        .config
                        .cache_dir
                        .join(format!("{}-{}", pkg.name, pkg.version));
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
                } else {
                    println!("{} {} is up-to-date", "OK".green(), pkg.name.cyan());
                }
            }
            Err(e) => {
                println!(
                    "{} {} update failed: {}",
                    "Failed".red(),
                    pkg.name.cyan(),
                    e
                );
                failed += 1;
            }
        }
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

#[cfg(test)]
mod tests {
    #[test]
    fn test_update_imports_compile() {
        assert!(true);
    }
}
