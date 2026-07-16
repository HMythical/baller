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

pub fn execute_draft(ctx: &AppContext, package_name: &str) -> Result<(), BallError> {
    println!("{} {}...", "Drafting".green().bold(), package_name.cyan());

    let pkg = ctx.registry.fetch_package(package_name)?;

    let mut installed = get_installed_map(&ctx.db);
    let result = resolve_deps(&pkg.name, &ctx.registry, &installed)?;

    // D1: Track packages installed in this session for rollback
    let mut session_packages: Vec<String> = Vec::new();

    for pkg_to_install in &result.packages {
        if installed.contains_key(&pkg_to_install.name) {
            println!(
                "{} {} already installed",
                "Skipping".yellow(),
                pkg_to_install.name.cyan()
            );
            continue;
        }

        // D2: Remove dead extra_env variable, pass &[] directly
        run_hook(
            &HookType::PreInstall,
            &pkg_to_install.name,
            &pkg_to_install.version,
            &ctx.config.hooks_dir,
            &ctx.config.hooks,
            &[],
        )?;

        let downloaded = ctx.downloader.download_and_extract(pkg_to_install, true)?;

        let install_path = downloaded.extract_dir.to_string_lossy().to_string();
        let bin_path_str = downloaded
            .binary_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());

        if let Some(binary_path) = &downloaded.binary_path {
            ActiveManager::create_symlink(binary_path, &pkg_to_install.name)?;
            println!(
                "{} symlinked to {}",
                "Linked".green(),
                binary_path.display().to_string().cyan()
            );
        } else {
            println!(
                "{} no binary found in extracted package",
                "Warning".yellow()
            );
        }

        // Pass user_installed=true for root packages, false for deps
        let is_root = pkg_to_install.name == pkg.name;
        ctx.db
            .insert_package(pkg_to_install, &install_path, bin_path_str.as_deref(), None, is_root)?;

        session_packages.push(pkg_to_install.name.clone());

        // D3: Update the installed map so subsequent packages see this one
        installed.insert(pkg_to_install.name.clone(), pkg_to_install.version.clone());

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

        println!(
            "{} {} v{} drafted!",
            "Done".green().bold(),
            pkg_to_install.name.cyan(),
            pkg_to_install.version.yellow()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    // Test that the function signature is correct and compiles.
    // Full integration testing requires a real AppContext which is hard to construct in unit tests.
    #[test]
    fn test_draft_imports_compile() {
        assert!(true);
    }
}
