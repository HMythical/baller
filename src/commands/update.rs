use colored::Colorize;

use crate::context::AppContext;
use crate::core::hooks::{run_hook, HookType};
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
                if remote_pkg.version != pkg.version {
                    println!(
                        "{} {}: {} -> {}",
                        "Updating".green(),
                        pkg.name.cyan(),
                        pkg.version.yellow(),
                        remote_pkg.version.green()
                    );

                    let pre_env = [("BALLER_NEW_VERSION", remote_pkg.version.as_str())];
                    run_hook(
                        &HookType::PreUpdate,
                        &pkg.name,
                        &pkg.version,
                        &ctx.config.hooks_dir,
                        &ctx.config.hooks,
                        &pre_env,
                    )?;

                    let downloaded = ctx.downloader.download_and_extract(&remote_pkg, true)?;

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
                    )?;

                    let post_env = [("BALLER_OLD_VERSION", pkg.version.as_str())];
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
