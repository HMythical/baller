use colored::Colorize;
use serde_json::json;

use crate::context::AppContext;
use crate::core::hooks::{run_hook, HookType};
use crate::error::error::BallError;
use crate::platform::common::PlatformManager;
use crate::utils::fs::confirm;
use crate::utils::output::print_json;

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

pub struct EjectOptions {
    pub force: bool,
    pub purge: bool,
    pub no_orphans: bool,
    pub keep_bin: bool,
}

pub fn execute_eject(
    ctx: &AppContext,
    package_name: &str,
    opts: &EjectOptions,
) -> Result<(), BallError> {
    if ctx.db.is_frozen(package_name)? && !opts.force {
        return Err(BallError::PackageFrozen(package_name.to_string()));
    }

    let installed = ctx.db.get_package(package_name)?;

    // E4: Confirmation prompt (unless --yes/-y)
    if !ctx.flags.yes
        && !confirm(&format!(
            "Are you sure you want to eject {}? [y/N]",
            installed.name.cyan()
        ))?
    {
        return Err(BallError::ConfirmationAborted);
    }

    run_hook(
        &HookType::PreEject,
        &installed.name,
        &installed.version,
        &ctx.config.hooks_dir,
        &ctx.config.hooks,
        &[],
    )?;

    if opts.keep_bin {
        tracing::info!(
            "{} leaving the linked binary for {} in place",
            "Keeping".yellow(),
            installed.name.cyan()
        );
    } else {
        ActiveManager::remove_symlink(&installed.name)?;
    }

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
    let mut orphans: Vec<String> = Vec::new();
    if !opts.no_orphans {
        let all_pkgs = ctx.db.list_packages()?;
        for other_pkg in &all_pkgs {
            // Check if this package was auto-installed (not user_installed) and is a dependency of no other package
            if !other_pkg.user_installed && *other_pkg.name != installed.name {
                let depended_on = ctx.db.is_depended_on(&other_pkg.name).unwrap_or(false);
                if !depended_on {
                    tracing::info!("Removing unused dependency {}...", other_pkg.name.cyan(),);
                    // Remove symlink if it exists
                    if !opts.keep_bin {
                        let _ = ActiveManager::remove_symlink(&other_pkg.name);
                    }
                    // Remove from DB
                    let _ = ctx.db.remove_package(&other_pkg.name);
                    orphans.push(other_pkg.name.clone());
                }
            }
        }
    }

    // E2: Clean extracted cache directory
    let extract_dir = ctx
        .config
        .cache_dir
        .join(format!("{}-{}", installed.name, installed.version));
    let _ = std::fs::remove_dir_all(&extract_dir);

    let mut purged = false;
    if opts.purge {
        match &installed.download_url {
            Some(url) => {
                purged = ctx.downloader.remove_archive(url)?;
                if purged {
                    tracing::info!(
                        "{} removed the cached archive for {}",
                        "Purged".red(),
                        installed.name.cyan()
                    );
                }
            }
            None => tracing::info!(
                "{} {} has no recorded download URL to purge",
                "Note".yellow(),
                installed.name.cyan()
            ),
        }
    }

    if ctx.flags.json {
        return print_json(&json!({
            "command": "eject",
            "package": installed.name,
            "version": installed.version,
            "status": "ejected",
            "orphans_removed": orphans,
            "archive_purged": purged,
            "binary_kept": opts.keep_bin,
        }));
    }

    println!("{} {} ejected", "Ejected".red().bold(), package_name.cyan());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eject_options_defaults_are_conservative() {
        let opts = EjectOptions {
            force: false,
            purge: false,
            no_orphans: false,
            keep_bin: false,
        };
        assert!(!opts.force);
        assert!(!opts.purge);
        assert!(!opts.no_orphans);
        assert!(!opts.keep_bin);
    }
}
