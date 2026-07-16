use colored::Colorize;

use crate::context::AppContext;
use crate::core::dep_solver::{get_installed_map, resolve_deps};
use crate::core::hooks::{run_hook, HookType};
use crate::error::error::BallError;
use crate::platform::common::PlatformManager;

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

pub fn execute_substitute(
    ctx: &AppContext,
    old_package: &str,
    new_package: &str,
) -> Result<(), BallError> {
    if ctx.db.is_frozen(old_package)? {
        return Err(BallError::PackageFrozen(old_package.to_string()));
    }

    println!(
        "{} Substituting {} with {}...",
        "Substituting".cyan(),
        old_package.cyan(),
        new_package.cyan()
    );

    let pkg = ctx.registry.fetch_package(new_package)?;

    // S1: Resolve dependencies for the new package
    let installed = get_installed_map(&ctx.db);
    let resolve_result = resolve_deps(&pkg.name, &ctx.registry, &installed)?;

    // S2: Run pre-install hooks for all packages first (before any changes)
    for dep_pkg in &resolve_result.packages {
        run_hook(
            &HookType::PreInstall,
            &dep_pkg.name,
            &dep_pkg.version,
            &ctx.config.hooks_dir,
            &ctx.config.hooks,
            &[],
        )?;
    }

    // Install all resolved packages (deps first, then root — already ordered by topological sort)
    let mut new_installed = None;
    for pkg_to_install in &resolve_result.packages {
        let downloaded = ctx.downloader.download_and_extract(pkg_to_install, true)?;

        let install_path = downloaded.extract_dir.to_string_lossy().to_string();
        let bin_path_str = downloaded
            .binary_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());

        if let Some(binary_path) = &downloaded.binary_path {
            ActiveManager::create_symlink(binary_path, &pkg_to_install.name)?;
        }

        let is_root = pkg_to_install.name == pkg.name;
        ctx.db.insert_package(
            pkg_to_install,
            &install_path,
            bin_path_str.as_deref(),
            None,
            is_root,
        )?;

        // S2: Run post-install hooks
        let bin_path_ref = bin_path_str.as_deref().unwrap_or("");
        let post_env = [
            ("BALLER_INSTALL_PATH", install_path.as_str()),
            ("BALLER_BIN_PATH", bin_path_ref),
        ];
        run_hook(
            &HookType::PostInstall,
            &pkg_to_install.name,
            &pkg_to_install.version,
            &ctx.config.hooks_dir,
            &ctx.config.hooks,
            &post_env,
        )?;

        // Track the newest root package for rollback info
        if pkg_to_install.name == pkg.name {
            new_installed = Some((install_path, bin_path_str));
        }
    }

    // Now remove the old package
    ActiveManager::remove_symlink(old_package)?;
    match ctx.db.remove_package(old_package) {
        Ok(_) => {}
        Err(BallError::PackageNotFound(_)) => {
            println!(
                "{} '{}' was not in the roster",
                "Note".yellow(),
                old_package.cyan()
            );
        }
        Err(e) => {
            // S3: Rollback on failure — remove the newly installed packages
            if let Some((ref install_path, ref _bin_path)) = new_installed {
                let _ = ctx.db.remove_package(&pkg.name);
                let _ = std::fs::remove_dir_all(install_path);
                println!(
                    "{} Rolled back substitution due to error: {}",
                    "Warning".red(),
                    e
                );
            }
            return Err(e);
        }
    }

    // S2: Run post-eject hook for old package
    run_hook(
        &HookType::PostEject,
        old_package,
        "",
        &ctx.config.hooks_dir,
        &ctx.config.hooks,
        &[],
    )?;

    println!(
        "{} Substitution complete: {} -> {}",
        "Done".green().bold(),
        old_package.cyan(),
        new_package.cyan()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_substitute_imports_compile() {
        assert!(true);
    }
}
