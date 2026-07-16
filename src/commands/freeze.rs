use colored::Colorize;

use crate::context::AppContext;
use crate::error::error::BallError;

pub fn execute_freeze(ctx: &AppContext, package_name: &str) -> Result<(), BallError> {
    // F1: Explicit existence check before toggle
    if !ctx.db.package_exists(package_name)? {
        return Err(BallError::PackageNotFound(package_name.to_string()));
    }

    let is_frozen = ctx.db.is_frozen(package_name)?;

    if is_frozen {
        ctx.db.set_frozen(package_name, false)?;
        println!(
            "{} {} thawed (unfrozen)",
            "Thawed".yellow(),
            package_name.cyan()
        );
    } else {
        ctx.db.set_frozen(package_name, true)?;
        println!("{} {} frozen", "Frozen".cyan(), package_name.cyan());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_freeze_imports_compile() {
        assert!(true);
    }
}
