use colored::Colorize;

use crate::context::AppContext;
use crate::error::error::BallError;

pub fn execute_build(_ctx: &AppContext, path: &str) -> Result<(), BallError> {
    println!(
        "{} Building from manifest at {}...",
        "Building".yellow(),
        path.cyan()
    );
    println!("{} Build command is not yet implemented", "Info".yellow());
    Ok(())
}
