use colored::Colorize;
use serde_json::json;

use crate::context::AppContext;
use crate::core::dep_solver::{get_installed_map, resolve_deps_with_root};
use crate::core::hooks::{run_hook, HookType};
use crate::core::package::{Package, PackageSource};
use crate::core::registry::RegistrySource;
use crate::error::error::BallError;
use crate::http::cargo::install_cargo_package;
use crate::http::system::install_system_package;
use crate::platform::common::PlatformManager;
use crate::utils::output::{info, print_json};

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

pub struct DraftOptions {
    pub version: Option<String>,
    pub source: Option<RegistrySource>,
    pub no_deps: bool,
    pub dry_run: bool,
    pub force: bool,
}

pub fn execute_draft(
    ctx: &AppContext,
    package_name: &str,
    opts: &DraftOptions,
) -> Result<(), BallError> {
    let quiet = ctx.flags.is_quiet();

    if opts.source == Some(RegistrySource::System) && !cfg!(target_os = "linux") {
        return Err(BallError::UnsupportedOs(
            "the system source is only available on Linux".to_string(),
        ));
    }

    info(
        quiet,
        format!("{} {}...", "Drafting".green().bold(), package_name.cyan()),
    );

    let pkg = fetch_root(ctx, package_name, opts)?;

    let mut installed = get_installed_map(&ctx.db);
    let result_packages = if opts.no_deps {
        vec![pkg.clone()]
    } else {
        resolve_deps_with_root(&pkg, &ctx.registry, &installed)?.packages
    };

    if opts.dry_run {
        return report_plan(ctx, &pkg, &result_packages, &installed, opts);
    }

    // D1: Track packages installed in this session for rollback
    let mut session_packages: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for pkg_to_install in &result_packages {
        if installed.contains_key(&pkg_to_install.name) && !opts.force {
            info(
                quiet,
                format!(
                    "{} {} already installed",
                    "Skipping".yellow(),
                    pkg_to_install.name.cyan()
                ),
            );
            skipped.push(pkg_to_install.name.clone());
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

        // System packages are installed via the native package manager,
        // not downloaded/archived. Record them in the DB and continue.
        if let PackageSource::System { manager } = &pkg_to_install.source {
            info(
                quiet,
                format!("{} installing via {}...", "System".green(), manager.cyan()),
            );
            install_system_package(manager, &pkg_to_install.name)?;

            let is_root = pkg_to_install.name == pkg.name;
            ctx.db
                .insert_package(pkg_to_install, "", None, None, is_root)?;
            session_packages.push(pkg_to_install.name.clone());
            installed.insert(pkg_to_install.name.clone(), pkg_to_install.version.clone());

            run_hook(
                &HookType::PostInstall,
                &pkg_to_install.name,
                &pkg_to_install.version,
                &ctx.config.hooks_dir,
                &ctx.config.hooks,
                &[],
            )?;

            info(
                quiet,
                format!(
                    "{} {} v{} installed via {}!",
                    "Done".green().bold(),
                    pkg_to_install.name.cyan(),
                    pkg_to_install.version.yellow(),
                    manager.cyan()
                ),
            );
            continue;
        }

        // Crates are compiled and installed by cargo into ~/.cargo/bin,
        // so there is nothing to download or extract here either.
        if let PackageSource::Cargo { crate_name } = &pkg_to_install.source {
            info(
                quiet,
                format!(
                    "{} installing {} via cargo...",
                    "Cargo".green(),
                    crate_name.cyan()
                ),
            );
            install_cargo_package(crate_name)?;

            let is_root = pkg_to_install.name == pkg.name;
            ctx.db
                .insert_package(pkg_to_install, "", None, None, is_root)?;
            session_packages.push(pkg_to_install.name.clone());
            installed.insert(pkg_to_install.name.clone(), pkg_to_install.version.clone());

            run_hook(
                &HookType::PostInstall,
                &pkg_to_install.name,
                &pkg_to_install.version,
                &ctx.config.hooks_dir,
                &ctx.config.hooks,
                &[],
            )?;

            info(
                quiet,
                format!(
                    "{} {} v{} installed via {}!",
                    "Done".green().bold(),
                    pkg_to_install.name.cyan(),
                    pkg_to_install.version.yellow(),
                    "cargo".cyan()
                ),
            );
            continue;
        }

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
            info(
                quiet,
                format!(
                    "{} symlinked to {}",
                    "Linked".green(),
                    binary_path.display().to_string().cyan()
                ),
            );
        } else {
            info(
                quiet,
                format!(
                    "{} no binary found in extracted package",
                    "Warning".yellow()
                ),
            );
        }

        // Pass user_installed=true for root packages, false for deps
        let is_root = pkg_to_install.name == pkg.name;
        ctx.db.insert_package(
            pkg_to_install,
            &install_path,
            bin_path_str.as_deref(),
            None,
            is_root,
        )?;

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

        info(
            quiet,
            format!(
                "{} {} v{} drafted!",
                "Done".green().bold(),
                pkg_to_install.name.cyan(),
                pkg_to_install.version.yellow()
            ),
        );
    }

    if ctx.flags.json {
        return print_json(&json!({
            "command": "draft",
            "package": pkg.name,
            "version": pkg.version,
            "source": source_label(&pkg.source),
            "installed": session_packages,
            "skipped": skipped,
        }));
    }

    Ok(())
}

/// Fetch the root package, honoring `--version` and `--source`
fn fetch_root(
    ctx: &AppContext,
    package_name: &str,
    opts: &DraftOptions,
) -> Result<Package, BallError> {
    match (opts.version.as_deref(), opts.source.as_ref()) {
        (Some(version), Some(source)) => {
            ctx.registry
                .fetch_package_at_version_from_source(source, package_name, version)
        }
        (Some(version), None) => ctx.registry.fetch_package_at_version(package_name, version),
        (None, Some(source)) => ctx.registry.fetch_package_from_source(source, package_name),
        (None, None) => ctx.registry.fetch_package(package_name),
    }
}

/// `--dry-run`: print the resolved plan and change nothing
fn report_plan(
    ctx: &AppContext,
    root: &Package,
    packages: &[Package],
    installed: &std::collections::HashMap<String, String>,
    opts: &DraftOptions,
) -> Result<(), BallError> {
    if ctx.flags.json {
        let entries: Vec<_> = packages
            .iter()
            .map(|pkg| {
                json!({
                    "name": pkg.name,
                    "version": pkg.version,
                    "source": source_label(&pkg.source),
                    "role": if pkg.name == root.name { "root" } else { "dependency" },
                    "action": plan_action(pkg, installed, opts),
                })
            })
            .collect();

        return print_json(&json!({
            "command": "draft",
            "package": root.name,
            "version": root.version,
            "dry_run": true,
            "plan": entries,
        }));
    }

    println!(
        "{} {} v{} from {}",
        "Dry run".yellow().bold(),
        root.name.cyan(),
        root.version.yellow(),
        source_label(&root.source).cyan()
    );

    for pkg in packages {
        let role = if pkg.name == root.name {
            "root"
        } else {
            "dependency"
        };
        println!(
            "  {} {} v{} ({}, {})",
            "•".cyan(),
            pkg.name.white().bold(),
            pkg.version.yellow(),
            role,
            plan_action(pkg, installed, opts)
        );
    }

    println!("{} nothing was installed", "Note".yellow());
    Ok(())
}

fn plan_action(
    pkg: &Package,
    installed: &std::collections::HashMap<String, String>,
    opts: &DraftOptions,
) -> &'static str {
    if !installed.contains_key(&pkg.name) {
        "install"
    } else if opts.force {
        "reinstall"
    } else {
        "skip — already installed"
    }
}

pub(crate) fn source_label(source: &PackageSource) -> String {
    match source {
        PackageSource::GitHub { owner, repo } => format!("github:{}/{}", owner, repo),
        PackageSource::BallerRegistry { .. } => "baller".to_string(),
        PackageSource::Chocolatey { .. } => "chocolatey".to_string(),
        PackageSource::System { manager } => format!("system:{}", manager),
        PackageSource::Cargo { crate_name } => format!("cargo:{}", crate_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn options() -> DraftOptions {
        DraftOptions {
            version: None,
            source: None,
            no_deps: false,
            dry_run: false,
            force: false,
        }
    }

    #[test]
    fn test_source_label_variants() {
        assert_eq!(
            source_label(&PackageSource::GitHub {
                owner: "owner".to_string(),
                repo: "repo".to_string()
            }),
            "github:owner/repo"
        );
        assert_eq!(
            source_label(&PackageSource::Chocolatey {
                feed_url: "feed".to_string()
            }),
            "chocolatey"
        );
        assert_eq!(
            source_label(&PackageSource::BallerRegistry {
                url: "url".to_string()
            }),
            "baller"
        );
        assert_eq!(
            source_label(&PackageSource::System {
                manager: "apt".to_string()
            }),
            "system:apt"
        );
    }

    #[test]
    fn test_plan_action_install() {
        let pkg = Package::new("fresh", "1.0.0");
        let installed = HashMap::new();
        assert_eq!(plan_action(&pkg, &installed, &options()), "install");
    }

    #[test]
    fn test_plan_action_skip_when_installed() {
        let pkg = Package::new("present", "1.0.0");
        let mut installed = HashMap::new();
        installed.insert("present".to_string(), "1.0.0".to_string());
        assert_eq!(
            plan_action(&pkg, &installed, &options()),
            "skip — already installed"
        );
    }

    #[test]
    fn test_plan_action_reinstall_with_force() {
        let pkg = Package::new("present", "1.0.0");
        let mut installed = HashMap::new();
        installed.insert("present".to_string(), "1.0.0".to_string());
        let opts = DraftOptions {
            force: true,
            ..options()
        };
        assert_eq!(plan_action(&pkg, &installed, &opts), "reinstall");
    }
}
