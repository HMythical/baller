use colored::Colorize;

use crate::context::AppContext;
use crate::error::error::BallError;

pub fn execute_freeze(ctx: &AppContext, package_name: &str) -> Result<(), BallError> {
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
