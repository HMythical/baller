use colored::Colorize;

use crate::context::AppContext;
use crate::error::error::BallError;
use crate::utils::fs as util_fs;

pub fn execute_sweep(ctx: &AppContext) -> Result<(), BallError> {
    let cache_dir = &ctx.config.cache_dir;

    if cache_dir.exists() {
        let size = util_fs::dir_size(cache_dir);
        println!("{} Clearing cache ({} bytes)...", "Sweeping".cyan(), size);
    } else {
        println!("{} Cache directory does not exist", "Sweeping".cyan());
    }

    ctx.downloader.cleanup_cache()?;

    println!("{} Cache cleaned", "Done".green().bold());
    Ok(())
}
