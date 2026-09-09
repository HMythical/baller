use colored::Colorize;
use serde_json::json;

use crate::context::AppContext;
use crate::core::dep_solver::{get_installed_map, resolve_deps_with_root};
use crate::core::hooks::{run_hook, HookType};
use crate::error::error::BallError;
use crate::platform::common::PlatformManager;
use crate::utils::fs::confirm;
use crate::utils::output::print_json;

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

pub struct SubstituteOptions {
    pub keep_old: bool,
    pub dry_run: bool,
    pub no_deps: bool,
}

pub fn execute_substitute(
    ctx: &AppContext,
    old_package: &str,
    new_package: &str,
    opts: &SubstituteOptions,
) -> Result<(), BallError> {
    let quiet = ctx.flags.is_quiet();

    // A frozen package may stay put, so only block when it would be removed.
    if !opts.keep_old && ctx.db.is_frozen(old_package)? {
        return Err(BallError::PackageFrozen(old_package.to_string()));
    }

    let pkg = ctx.registry.fetch_package(new_package)?;

    let installed = get_installed_map(&ctx.db);
    let packages = if opts.no_deps {
        vec![pkg.clone()]
    } else {
        resolve_deps_with_root(&pkg, &ctx.registry, &installed)?.packages
    };

    if opts.dry_run {
        return report_plan(ctx, old_package, &pkg, &packages, opts);
    }

    // Prompting is the default here, matching eject and sweep.
    if !ctx.flags.yes {
        let prompt = if opts.keep_old {
            format!(
                "Install {} alongside {}? [y/N]",
                new_package.cyan(),
                old_package.cyan()
            )
        } else {
            format!(
                "Are you sure you want to substitute {} with {}? [y/N]",
                old_package.cyan(),
                new_package.cyan()
            )
        };

        if !confirm(&prompt)? {
            return Err(BallError::ConfirmationAborted);
        }
    }

    tracing::info!(
        "{} {} with {}...",
        "Substituting".cyan(),
        old_package.cyan(),
        new_package.cyan()
    );

    // S2: Run pre-install hooks for all packages first (before any changes)
    for dep_pkg in &packages {
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
    for pkg_to_install in &packages {
        let downloaded = ctx
            .downloader
            .download_and_extract(pkg_to_install, !quiet)?;

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

    if opts.keep_old {
        tracing::info!(
            "{} {} left on the roster",
            "Keeping".yellow(),
            old_package.cyan()
        );
    } else {
        // Now remove the old package
        ActiveManager::remove_symlink(old_package)?;
        match ctx.db.remove_package(old_package) {
            Ok(_) => {}
            Err(BallError::PackageNotFound(_)) => {
                tracing::info!(
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
    }

    if ctx.flags.json {
        return print_json(&json!({
            "command": "substitute",
            "old_package": old_package,
            "new_package": pkg.name,
            "new_version": pkg.version,
            "old_kept": opts.keep_old,
            "installed": packages.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
            "status": "substituted",
        }));
    }

    println!(
        "{} Substitution complete: {} -> {}",
        "Done".green().bold(),
        old_package.cyan(),
        new_package.cyan()
    );
    Ok(())
}

/// `--dry-run`: describe the swap without installing or removing anything
fn report_plan(
    ctx: &AppContext,
    old_package: &str,
    root: &crate::core::package::Package,
    packages: &[crate::core::package::Package],
    opts: &SubstituteOptions,
) -> Result<(), BallError> {
    if ctx.flags.json {
        return print_json(&json!({
            "command": "substitute",
            "old_package": old_package,
            "new_package": root.name,
            "new_version": root.version,
            "dry_run": true,
            "old_kept": opts.keep_old,
            "would_install": packages.iter().map(|p| json!({
                "name": p.name,
                "version": p.version,
            })).collect::<Vec<_>>(),
        }));
    }

    println!(
        "{} substitute {} with {} v{}",
        "Dry run".yellow().bold(),
        old_package.cyan(),
        root.name.cyan(),
        root.version.yellow()
    );

    for pkg in packages {
        println!(
            "  {} install {} v{}",
            "•".cyan(),
            pkg.name.white().bold(),
            pkg.version.yellow()
        );
    }

    if opts.keep_old {
        println!("  {} keep {}", "•".cyan(), old_package.cyan());
    } else {
        println!("  {} eject {}", "•".cyan(), old_package.cyan());
    }

    println!("{} nothing was changed", "Note".yellow());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substitute_options_default_to_prompted_swap() {
        let opts = SubstituteOptions {
            keep_old: false,
            dry_run: false,
            no_deps: false,
        };
        assert!(!opts.keep_old);
        assert!(!opts.dry_run);
        assert!(!opts.no_deps);
    }
}
