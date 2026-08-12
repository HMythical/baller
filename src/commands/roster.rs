use colored::Colorize;
use serde_json::json;

use crate::context::AppContext;
use crate::core::db::InstalledPackage;
use crate::core::dep_solver::parse_version_flexible;
use crate::core::registry::RegistrySource;
use crate::error::error::BallError;
use crate::utils::fs::truncate_str;
use crate::utils::output::print_json;

pub struct RosterOptions {
    pub frozen: bool,
    pub source: Option<RegistrySource>,
    pub outdated: bool,
    pub remote: bool,
}

pub fn execute_roster(
    ctx: &AppContext,
    package_name: &Option<String>,
    opts: &RosterOptions,
) -> Result<(), BallError> {
    if opts.remote {
        let query = package_name
            .as_deref()
            .ok_or_else(|| BallError::InvalidConfig("--remote needs a search term".to_string()))?;
        return search_remote(ctx, query);
    }

    if let Some(pkg_name) = package_name {
        return match ctx.db.get_package(pkg_name) {
            Ok(pkg) => show_detail(ctx, &pkg),
            Err(_) => {
                println!(
                    "{} '{}' not found locally, searching registries...",
                    "Searching".cyan(),
                    pkg_name.cyan()
                );
                search_remote(ctx, pkg_name)
            }
        };
    }

    list_packages(ctx, opts)
}

fn list_packages(ctx: &AppContext, opts: &RosterOptions) -> Result<(), BallError> {
    let mut pkgs = ctx.db.list_packages()?;

    if opts.frozen {
        pkgs.retain(|pkg| pkg.frozen);
    }

    if let Some(source) = &opts.source {
        let wanted = source.db_name();
        pkgs.retain(|pkg| pkg.source == wanted);
    }

    if opts.outdated {
        return report_outdated(ctx, &pkgs);
    }

    if ctx.flags.json {
        return print_json(&json!({
            "command": "roster",
            "count": pkgs.len(),
            "packages": pkgs,
        }));
    }

    if pkgs.is_empty() {
        if opts.frozen || opts.source.is_some() {
            println!("{} No packages match that filter", "Empty".yellow());
        } else {
            println!(
                "{} No packages installed. Use 'baller draft <name>' to install.",
                "Empty".yellow()
            );
        }
        return Ok(());
    }

    println!(
        "\n{} Active Roster ({}):",
        "Roster".cyan(),
        format!("{} players", pkgs.len()).cyan()
    );
    println!("{}", "─".repeat(60));

    for pkg in &pkgs {
        if ctx.flags.verbose > 0 {
            print_detail_block(ctx, pkg)?;
            continue;
        }

        let freeze_tag = if pkg.frozen { " ❄️" } else { "" };
        println!(
            "  {} {} v{}{}",
            "•".cyan(),
            pkg.name.white().bold(),
            pkg.version.yellow(),
            freeze_tag
        );
        if let Some(desc) = &pkg.description {
            // R1 & R3: Use safe character-based truncation via truncate_str helper
            println!("    {}", truncate_str(desc, 50));
        }
    }

    println!("{}", "─".repeat(60));
    Ok(())
}

fn show_detail(ctx: &AppContext, pkg: &InstalledPackage) -> Result<(), BallError> {
    if ctx.flags.json {
        let deps = ctx.db.get_dependencies(&pkg.name)?;
        return print_json(&json!({
            "command": "roster",
            "package": pkg,
            "dependencies": deps.iter().map(|(name, version)| json!({
                "name": name,
                "version": version,
            })).collect::<Vec<_>>(),
        }));
    }

    println!("\n{} - {}", "Roster".cyan(), pkg.name.cyan().bold());
    print_detail_fields(pkg);

    let deps = ctx.db.get_dependencies(&pkg.name)?;
    if !deps.is_empty() {
        println!("  {}:", "Dependencies".yellow());
        for (dep_name, dep_ver) in &deps {
            println!("    - {} {}", dep_name.cyan(), dep_ver);
        }
    }

    Ok(())
}

/// The detail view reused by `--verbose` in list mode
fn print_detail_block(ctx: &AppContext, pkg: &InstalledPackage) -> Result<(), BallError> {
    println!("  {} {}", "•".cyan(), pkg.name.cyan().bold());
    print_detail_fields(pkg);

    let deps = ctx.db.get_dependencies(&pkg.name)?;
    if !deps.is_empty() {
        println!("  {}:", "Dependencies".yellow());
        for (dep_name, dep_ver) in &deps {
            println!("    - {} {}", dep_name.cyan(), dep_ver);
        }
    }

    Ok(())
}

fn print_detail_fields(pkg: &InstalledPackage) {
    println!("  {} {}", "Version:".yellow(), pkg.version);
    println!("  {} {}", "Source:".yellow(), pkg.source);
    if let Some(detail) = &pkg.source_detail {
        println!("  {} {}", "Source Detail:".yellow(), detail);
    }
    if let Some(desc) = &pkg.description {
        // R3: Use truncate_str helper for safe truncation in detail view too
        println!("  {} {}", "Description:".yellow(), truncate_str(desc, 80));
    }
    if let Some(author) = &pkg.author {
        println!("  {} {}", "Author:".yellow(), author);
    }
    println!(
        "  {} {}",
        "Frozen:".yellow(),
        if pkg.frozen {
            "yes".red()
        } else {
            "no".green()
        }
    );
    println!("  {} {}", "Install Path:".yellow(), pkg.install_path);
    if let Some(bp) = &pkg.bin_path {
        println!("  {} {}", "Binary:".yellow(), bp);
    }
}

/// `--outdated`: compare installed versions against the registries
fn report_outdated(ctx: &AppContext, pkgs: &[InstalledPackage]) -> Result<(), BallError> {
    let mut stale: Vec<serde_json::Value> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();

    for pkg in pkgs {
        match ctx.registry.fetch_package(&pkg.name) {
            Ok(remote) => {
                let installed = parse_version_flexible(&pkg.version);
                let available = parse_version_flexible(&remote.version);
                let is_stale = match (installed, available) {
                    (Some(cur), Some(rem)) => rem > cur,
                    _ => remote.version != pkg.version,
                };

                if is_stale {
                    stale.push(json!({
                        "name": pkg.name,
                        "installed": pkg.version,
                        "available": remote.version,
                        "frozen": pkg.frozen,
                    }));
                }
            }
            Err(_) => unknown.push(pkg.name.clone()),
        }
    }

    if ctx.flags.json {
        return print_json(&json!({
            "command": "roster",
            "checked": pkgs.len(),
            "outdated": stale,
            "unresolved": unknown,
        }));
    }

    if stale.is_empty() {
        println!("{} Every package is up-to-date", "OK".green());
    } else {
        println!("\n{} Outdated ({}):", "Roster".cyan(), stale.len());
        println!("{}", "─".repeat(60));
        for entry in &stale {
            let frozen_tag = if entry["frozen"].as_bool().unwrap_or(false) {
                " ❄️"
            } else {
                ""
            };
            println!(
                "  {} {} {} -> {}{}",
                "•".cyan(),
                entry["name"].as_str().unwrap_or("").white().bold(),
                entry["installed"].as_str().unwrap_or("").yellow(),
                entry["available"].as_str().unwrap_or("").green(),
                frozen_tag
            );
        }
        println!("{}", "─".repeat(60));
    }

    if !unknown.is_empty() {
        println!(
            "{} could not check {}",
            "Note".yellow(),
            unknown.join(", ").cyan()
        );
    }

    Ok(())
}

fn search_remote(ctx: &AppContext, query: &str) -> Result<(), BallError> {
    let results = ctx.registry.search(query)?;

    if ctx.flags.json {
        return print_json(&json!({
            "command": "roster",
            "query": query,
            "results": results.iter().map(|pkg| json!({
                "name": pkg.name,
                "version": pkg.version,
                "description": pkg.description,
            })).collect::<Vec<_>>(),
        }));
    }

    if results.is_empty() {
        println!(
            "{} No packages found matching '{}'",
            "Not found".red(),
            query.cyan()
        );
        return Ok(());
    }

    println!(
        "\n{} Remote results for '{}':",
        "Results".cyan(),
        query.cyan()
    );
    // R2: Show description in remote search results
    for pkg in &results {
        println!(
            "  {} {} {}",
            "•".cyan(),
            pkg.name.white().bold(),
            format!("v{}", pkg.version).yellow()
        );
        if let Some(desc) = &pkg.description {
            println!("    {}", desc);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_filter_uses_db_names() {
        assert_eq!(RegistrySource::GitHub.db_name(), "github");
        assert_eq!(RegistrySource::BallerRegistry.db_name(), "baller_registry");
        assert_eq!(RegistrySource::Chocolatey.db_name(), "chocolatey");
        assert_eq!(RegistrySource::System.db_name(), "system");
    }

    #[test]
    fn test_roster_options_default_to_plain_list() {
        let opts = RosterOptions {
            frozen: false,
            source: None,
            outdated: false,
            remote: false,
        };
        assert!(!opts.frozen);
        assert!(opts.source.is_none());
        assert!(!opts.outdated);
        assert!(!opts.remote);
    }
}
