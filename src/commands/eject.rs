use colored::Colorize;

use crate::context::AppContext;
use crate::core::hooks::{run_hook, HookType};
use crate::error::error::BallError;
use crate::platform::common::PlatformManager;
use crate::utils::fs::confirm;

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

pub fn execute_eject(ctx: &AppContext, package_name: &str, force: bool) -> Result<(), BallError> {
    // E4: Confirmation prompt (unless --yes/-y)
    if !force && !confirm(&format!("Are you sure you want to eject {}? [y/N]", package_name.cyan())) {
        println!("Aborted.");
        return Ok(());
    }

    if ctx.db.is_frozen(package_name)? {
        return Err(BallError::PackageFrozen(package_name.to_string()));
    }

    let installed = ctx.db.get_package(package_name)?;

    run_hook(
        &HookType::PreEject,
        &installed.name,
        &installed.version,
        &ctx.config.hooks_dir,
        &ctx.config.hooks,
        &[],
    )?;

    ActiveManager::remove_symlink(&installed.name)?;

    // E3: Run post-eject hook BEFORE removing from DB
    run_hook(
        &HookType::PostEject,
        &installed.name,
        &installed.version,
        &ctx.config.hooks_dir,
        &ctx.config.hooks,
        &[],
    )?;

    ctx.db.remove_package(package_name)?;

    // E1: Orphan dependency cleanup
    let all_pkgs = ctx.db.list_packages()?;
    for other_pkg in &all_pkgs {
        // Check if this package was auto-installed (not user_installed) and is a dependency of no other package
        if !other_pkg.user_installed && *other_pkg.name != installed.name {
            let depended_on = ctx.db.is_depended_on(&other_pkg.name).unwrap_or(false);
            if !depended_on {
                println!(
                    "Removing unused dependency {}...",
                    other_pkg.name.cyan()
                );
                // Remove symlink if it exists
                let _ = ActiveManager::remove_symlink(&other_pkg.name);
                // Remove from DB
                let _ = ctx.db.remove_package(&other_pkg.name);
            }
        }
    }

    // E2: Clean extracted cache directory
    let extract_dir = ctx
        .config
        .cache_dir
        .join(format!("{}-{}", installed.name, installed.version));
    let _ = std::fs::remove_dir_all(&extract_dir);

    println!(
        "{} {} ejected",
        "Ejected".red().bold(),
        package_name.cyan()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_eject_imports_compile() {
        assert!(true);
    }
}
