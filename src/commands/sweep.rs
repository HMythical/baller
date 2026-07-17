use colored::Colorize;

use crate::context::AppContext;
use crate::error::error::BallError;
use crate::utils::fs as util_fs;
use crate::utils::fs::{confirm, format_size};

pub fn execute_sweep(ctx: &AppContext, force: bool) -> Result<(), BallError> {
    let cache_dir = &ctx.config.cache_dir;

    if cache_dir.exists() && util_fs::dir_size(cache_dir) > 0 {
        let size = util_fs::dir_size(cache_dir);
        let formatted = format_size(size);

        // SW2: Confirmation prompt unless --yes/-y
        if !force
            && !confirm(&format!(
                "Are you sure you want to clear {} of cached packages? [y/N]",
                formatted.green()
            ))
        {
            println!("Aborted.");
            return Ok(());
        } else {
            println!(
                "{} Clearing cache ({})...",
                "Sweeping".cyan(),
                formatted.green()
            );
        }
    } else {
        println!(
            "{} Cache directory does not exist or is empty",
            "Sweeping".cyan()
        );
    }

    ctx.downloader.cleanup_cache()?;

    println!("{} Cache cleaned", "Done".green().bold());
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::utils::fs::format_size;

    #[test]
    fn test_sweep_imports_compile() {
        assert!(true);
    }

    #[test]
    fn test_format_size_zero() {
        assert_eq!(format_size(0), "0 bytes");
    }

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(500), "500 bytes");
    }

    #[test]
    fn test_format_size_kb() {
        let result = format_size(1536); // 1.5 KB
        assert!(result.contains("KB"));
        assert!(result.contains("1.5"));
    }

    #[test]
    fn test_format_size_mb() {
        let result = format_size(10_485_760); // ~10 MB
        assert!(result.contains("MB"));
        assert!(result.contains("10"));
    }

    #[test]
    fn test_format_size_gb() {
        let result = format_size(2_147_483_648); // 2 GB
        assert!(result.contains("GB"));
        assert!(result.contains("2"));
    }
}
