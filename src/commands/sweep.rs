use colored::Colorize;
use serde_json::json;
use std::path::PathBuf;

use crate::context::AppContext;
use crate::error::error::BallError;
use crate::utils::fs as util_fs;
use crate::utils::fs::{confirm, format_size, parse_size};
use crate::utils::output::print_json;

pub struct SweepOptions {
    pub all: bool,
    pub dry_run: bool,
    pub threshold: Option<String>,
}

pub fn execute_sweep(ctx: &AppContext, opts: &SweepOptions) -> Result<(), BallError> {
    let cache_dir = &ctx.config.cache_dir;
    let json = ctx.flags.json;

    let archives = ctx.downloader.archive_paths();
    let extracted = extracted_dirs(cache_dir);
    let archive_bytes: u64 = archives
        .iter()
        .filter_map(|path| path.metadata().ok())
        .map(|meta| meta.len())
        .sum();
    let cache_bytes = if cache_dir.exists() {
        util_fs::dir_size(cache_dir)
    } else {
        0
    };

    let mode = if opts.all { "all" } else { "archives" };
    let targeted_bytes = if opts.all { cache_bytes } else { archive_bytes };
    let extracted_targeted = if opts.all { extracted.len() } else { 0 };

    if let Some(raw) = &opts.threshold {
        let threshold = parse_size(raw)?;
        if cache_bytes <= threshold {
            if json {
                return print_json(&json!({
                    "command": "sweep",
                    "mode": mode,
                    "status": "below-threshold",
                    "cache_bytes": cache_bytes,
                    "threshold_bytes": threshold,
                }));
            }
            println!(
                "{} cache is {}, below the {} threshold — nothing swept",
                "Sweeping".cyan(),
                format_size(cache_bytes).green(),
                format_size(threshold).green()
            );
            return Ok(());
        }
    }

    if archives.is_empty() && (!opts.all || extracted.is_empty()) {
        if json {
            return print_json(&json!({
                "command": "sweep",
                "mode": mode,
                "status": "empty",
                "cache_bytes": cache_bytes,
            }));
        }
        println!(
            "{} Cache directory does not exist or is empty",
            "Sweeping".cyan()
        );
        return Ok(());
    }

    if opts.dry_run {
        if json {
            return print_json(&json!({
                "command": "sweep",
                "mode": mode,
                "status": "would-sweep",
                "archives": archives.len(),
                "extracted": extracted_targeted,
                "bytes": targeted_bytes,
                "cache_bytes": cache_bytes,
            }));
        }

        println!(
            "{} would remove {} ({})",
            "Sweeping".cyan(),
            describe_targets(archives.len(), extracted_targeted),
            format_size(targeted_bytes).green()
        );
        for archive in &archives {
            println!("  {} {}", "•".cyan(), archive.display());
        }
        if opts.all {
            for dir in &extracted {
                println!("  {} {}{}", "•".cyan(), dir.display(), "/".dimmed());
            }
        }
        return Ok(());
    }

    if !ctx.flags.yes {
        let prompt = if opts.all {
            format!(
                "Are you sure you want to clear {} of cache, including extracted packages? [y/N]",
                format_size(targeted_bytes).green()
            )
        } else {
            format!(
                "Are you sure you want to clear {} of cached archives? [y/N]",
                format_size(targeted_bytes).green()
            )
        };

        if !confirm(&prompt) {
            if json {
                return print_json(&json!({
                    "command": "sweep",
                    "mode": mode,
                    "status": "aborted",
                }));
            }
            println!("Aborted.");
            return Ok(());
        }
    }

    let (removed, freed) = if opts.all {
        ctx.downloader.cleanup_all()?;
        (archives.len() as u64, cache_bytes)
    } else {
        ctx.downloader.cleanup_archives()?
    };

    if json {
        return print_json(&json!({
            "command": "sweep",
            "mode": mode,
            "status": "swept",
            "archives": removed,
            "extracted": extracted_targeted,
            "freed_bytes": freed,
        }));
    }

    println!(
        "{} Cleared {} ({})",
        "Done".green().bold(),
        describe_targets(removed as usize, extracted_targeted),
        format_size(freed).green()
    );

    if !opts.all && !extracted.is_empty() {
        println!(
            "{} kept {} extracted package(s) — pass {} to remove them too",
            "Note".yellow(),
            extracted.len().to_string().cyan(),
            "--all".cyan()
        );
    }

    Ok(())
}

/// Extracted `<name>-<version>` directories, which hold the linked binaries
fn extracted_dirs(cache_dir: &std::path::Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(cache_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            }
        }
    }
    dirs.sort();
    dirs
}

fn describe_targets(archives: usize, extracted: usize) -> String {
    if extracted > 0 {
        format!(
            "{} archive(s) and {} extracted package(s)",
            archives, extracted
        )
    } else {
        format!("{} archive(s)", archives)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::fs::format_size;

    fn temp_dir(tag: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("baller_test_sweep_{}_{}", tag, nanos));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_extracted_dirs_lists_only_directories() {
        let dir = temp_dir("extracted");
        std::fs::write(dir.join("archive.zip"), b"zip").unwrap();
        std::fs::create_dir_all(dir.join("pkg-1.0.0")).unwrap();
        std::fs::create_dir_all(dir.join("other-2.0.0")).unwrap();

        let dirs = extracted_dirs(&dir);
        assert_eq!(dirs.len(), 2);
        assert!(dirs.iter().all(|p| p.is_dir()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extracted_dirs_missing_cache() {
        let dirs = extracted_dirs(std::path::Path::new("/nonexistent/baller/cache"));
        assert!(dirs.is_empty());
    }

    #[test]
    fn test_describe_targets() {
        assert_eq!(describe_targets(3, 0), "3 archive(s)");
        assert_eq!(
            describe_targets(3, 2),
            "3 archive(s) and 2 extracted package(s)"
        );
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
