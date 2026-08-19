use colored::Colorize;
use serde_json::json;

use crate::context::AppContext;
use crate::error::error::BallError;
use crate::utils::output::print_json;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FreezeMode {
    Toggle,
    Freeze,
    Thaw,
}

impl FreezeMode {
    /// `--freeze` and `--thaw` are mutually exclusive; neither means toggle
    pub fn from_flags(freeze: bool, thaw: bool) -> Self {
        match (freeze, thaw) {
            (true, _) => FreezeMode::Freeze,
            (_, true) => FreezeMode::Thaw,
            _ => FreezeMode::Toggle,
        }
    }

    /// The state to write, given what the package is set to now
    fn target(&self, currently_frozen: bool) -> bool {
        match self {
            FreezeMode::Toggle => !currently_frozen,
            FreezeMode::Freeze => true,
            FreezeMode::Thaw => false,
        }
    }
}

pub struct FreezeOptions {
    pub mode: FreezeMode,
    pub all: bool,
    pub list: bool,
}

pub fn execute_freeze(
    ctx: &AppContext,
    package_name: &Option<String>,
    opts: &FreezeOptions,
) -> Result<(), BallError> {
    if opts.list {
        return list_frozen(ctx);
    }

    if opts.all {
        if opts.mode == FreezeMode::Toggle {
            return Err(BallError::InvalidConfig(
                "--all needs an explicit direction: pass --freeze or --thaw".to_string(),
            ));
        }
        return apply_to_all(ctx, opts.mode);
    }

    let package_name = package_name.as_deref().ok_or_else(|| {
        BallError::InvalidConfig("specify a package name, or use --all or --list".to_string())
    })?;

    // F1: Explicit existence check before toggle
    if !ctx.db.package_exists(package_name)? {
        return Err(BallError::PackageNotFound(package_name.to_string()));
    }

    let is_frozen = ctx.db.is_frozen(package_name)?;
    let target = opts.mode.target(is_frozen);

    if target == is_frozen {
        if ctx.flags.json {
            return print_json(&json!({
                "command": "freeze",
                "package": package_name,
                "frozen": is_frozen,
                "status": "unchanged",
            }));
        }

        println!(
            "{} {} is already {}",
            "Note".yellow(),
            package_name.cyan(),
            if is_frozen { "frozen" } else { "thawed" }
        );
        return Ok(());
    }

    ctx.db.set_frozen(package_name, target)?;

    if ctx.flags.json {
        return print_json(&json!({
            "command": "freeze",
            "package": package_name,
            "frozen": target,
            "status": if target { "frozen" } else { "thawed" },
        }));
    }

    if target {
        println!("{} {} frozen", "Frozen".cyan(), package_name.cyan());
    } else {
        println!(
            "{} {} thawed (unfrozen)",
            "Thawed".yellow(),
            package_name.cyan()
        );
    }

    Ok(())
}

/// `--all`: drive every installed package to the same state
fn apply_to_all(ctx: &AppContext, mode: FreezeMode) -> Result<(), BallError> {
    let pkgs = ctx.db.list_packages()?;
    let target = matches!(mode, FreezeMode::Freeze);
    let mut changed: Vec<String> = Vec::new();

    for pkg in &pkgs {
        if pkg.frozen == target {
            continue;
        }
        ctx.db.set_frozen(&pkg.name, target)?;
        changed.push(pkg.name.clone());
    }

    if ctx.flags.json {
        return print_json(&json!({
            "command": "freeze",
            "scope": "all",
            "frozen": target,
            "changed": changed,
            "total": pkgs.len(),
        }));
    }

    if changed.is_empty() {
        println!(
            "{} every package is already {}",
            "Note".yellow(),
            if target { "frozen" } else { "thawed" }
        );
        return Ok(());
    }

    for name in &changed {
        if target {
            println!("{} {} frozen", "Frozen".cyan(), name.cyan());
        } else {
            println!("{} {} thawed (unfrozen)", "Thawed".yellow(), name.cyan());
        }
    }

    println!(
        "{} {} package(s) {}",
        "Done".green().bold(),
        changed.len(),
        if target { "frozen" } else { "thawed" }
    );
    Ok(())
}

/// `--list`: show what is currently pinned
fn list_frozen(ctx: &AppContext) -> Result<(), BallError> {
    let frozen = ctx.db.list_frozen()?;

    if ctx.flags.json {
        return print_json(&json!({
            "command": "freeze",
            "frozen": frozen,
        }));
    }

    if frozen.is_empty() {
        println!("{} No frozen packages", "Empty".yellow());
        return Ok(());
    }

    println!("\n{} Frozen ({}):", "Roster".cyan(), frozen.len());
    println!("{}", "─".repeat(60));
    for pkg in &frozen {
        println!(
            "  {} {} v{} ❄️",
            "•".cyan(),
            pkg.name.white().bold(),
            pkg.version.yellow()
        );
    }
    println!("{}", "─".repeat(60));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_freeze_mode_from_flags() {
        assert_eq!(FreezeMode::from_flags(false, false), FreezeMode::Toggle);
        assert_eq!(FreezeMode::from_flags(true, false), FreezeMode::Freeze);
        assert_eq!(FreezeMode::from_flags(false, true), FreezeMode::Thaw);
    }

    #[test]
    fn test_toggle_target_flips() {
        assert!(FreezeMode::Toggle.target(false));
        assert!(!FreezeMode::Toggle.target(true));
    }

    #[test]
    fn test_directional_targets_are_absolute() {
        assert!(FreezeMode::Freeze.target(false));
        assert!(FreezeMode::Freeze.target(true));
        assert!(!FreezeMode::Thaw.target(false));
        assert!(!FreezeMode::Thaw.target(true));
    }
}
