use colored::Colorize;

use crate::context::AppContext;
use crate::error::error::BallError;
use crate::utils::fs::truncate_str;

pub fn execute_roster(ctx: &AppContext, package_name: &Option<String>) -> Result<(), BallError> {
    if let Some(pkg_name) = package_name {
        match ctx.db.get_package(pkg_name) {
            Ok(pkg) => {
                println!("\n{} - {}", "Roster".cyan(), pkg.name.cyan().bold());
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

                let deps = ctx.db.get_dependencies(pkg_name)?;
                if !deps.is_empty() {
                    println!("  {}:", "Dependencies".yellow());
                    for (dep_name, dep_ver) in &deps {
                        println!("    - {} {}", dep_name.cyan(), dep_ver);
                    }
                }
            }
            Err(_) => {
                println!(
                    "{} '{}' not found locally, searching registries...",
                    "Searching".cyan(),
                    pkg_name.cyan()
                );
                let results = ctx.registry.search(pkg_name)?;
                if results.is_empty() {
                    println!(
                        "{} No packages found matching '{}'",
                        "Not found".red(),
                        pkg_name.cyan()
                    );
                } else {
                    println!(
                        "\n{} Remote results for '{}':",
                        "Results".cyan(),
                        pkg_name.cyan()
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
                }
            }
        }
    } else {
        let pkgs = ctx.db.list_packages()?;
        if pkgs.is_empty() {
            println!(
                "{} No packages installed. Use 'baller draft <name>' to install.",
                "Empty".yellow()
            );
        } else {
            println!(
                "\n{} Active Roster ({}):",
                "Roster".cyan(),
                format!("{} players", pkgs.len()).cyan()
            );
            println!("{}", "─".repeat(60));
            for pkg in &pkgs {
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
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_roster_imports_compile() {
        assert!(true);
    }
}
