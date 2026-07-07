use colored::Colorize;

use crate::context::AppContext;
use crate::core::hooks::{run_hook, HookType};
use crate::error::error::BallError;
use crate::platform::common::PlatformManager;

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

pub fn execute_eject(ctx: &AppContext, package_name: &str) -> Result<(), BallError> {
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

    ctx.db.remove_package(package_name)?;

    run_hook(
        &HookType::PostEject,
        &installed.name,
        &installed.version,
        &ctx.config.hooks_dir,
        &ctx.config.hooks,
        &[],
    )?;

    println!("{} {} ejected", "Ejected".red().bold(), package_name.cyan());
    Ok(())
}
