use colored::Colorize;

use crate::context::AppContext;
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

    let downloaded = ctx.downloader.download_and_extract(&pkg, true)?;

    let install_path = downloaded.extract_dir.to_string_lossy().to_string();
    let bin_path_str = downloaded
        .binary_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    if let Some(binary_path) = &downloaded.binary_path {
        ActiveManager::create_symlink(binary_path, &pkg.name)?;
    }

    ctx.db
        .insert_package(&pkg, &install_path, bin_path_str.as_deref(), None)?;

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
        Err(e) => return Err(e),
    }

    println!(
        "{} Substitution complete: {} -> {}",
        "Done".green().bold(),
        old_package.cyan(),
        new_package.cyan()
    );
    Ok(())
}
