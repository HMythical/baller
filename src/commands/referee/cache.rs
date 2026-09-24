//! `baller referee cache` — the verdict cache as a first-class surface.
//!
//! Works with Referee switched off: managing stored verdicts asks nothing of
//! the advisory service.

use colored::Colorize;
use serde_json::json;

use crate::context::AppContext;
use crate::error::error::BallError;
use crate::utils::output::print_json;

/// What `baller referee cache` does. `Status` is the default.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CacheAction {
    Status,
    Clear,
    /// Drop verdicts computed more than this many days ago
    Prune(u32),
}

pub fn execute_cache(ctx: &AppContext, action: CacheAction) -> Result<(), BallError> {
    match action {
        CacheAction::Status => status(ctx),
        CacheAction::Clear => {
            let removed = ctx.db.referee_cache_clear()?;
            if ctx.flags.json {
                return print_json(&json!({
                    "command": "referee",
                    "subcommand": "cache",
                    "action": "clear",
                    "removed": removed,
                }));
            }
            println!("{} {} cached verdict(s)", "Cleared".green().bold(), removed);
            Ok(())
        }
        CacheAction::Prune(days) => {
            let removed = ctx.db.referee_cache_prune_older_than(days)?;
            if ctx.flags.json {
                return print_json(&json!({
                    "command": "referee",
                    "subcommand": "cache",
                    "action": "prune",
                    "days": days,
                    "removed": removed,
                }));
            }
            println!(
                "{} {} cached verdict(s) older than {} day(s)",
                "Pruned".green().bold(),
                removed,
                days
            );
            Ok(())
        }
    }
}

fn status(ctx: &AppContext) -> Result<(), BallError> {
    let stats = ctx.db.referee_cache_stats()?;
    let total: i64 = stats.iter().map(|row| row.count).sum();
    // `checked_at` is `YYYY-MM-DD HH:MM:SS`, so the string maximum is the newest.
    let newest = stats.iter().filter_map(|row| row.newest.clone()).max();

    if ctx.flags.json {
        return print_json(&json!({
            "command": "referee",
            "subcommand": "cache",
            "action": "status",
            "total": total,
            "newest": newest,
            "ecosystems": stats
                .iter()
                .map(|row| json!({
                    "ecosystem": row.ecosystem,
                    "count": row.count,
                    "newest": row.newest,
                }))
                .collect::<Vec<_>>(),
        }));
    }

    if total == 0 {
        println!("{} the verdict cache is empty", "Note".yellow());
        return Ok(());
    }

    println!(
        "{:<16} {:<8} {}",
        "ECOSYSTEM".bold(),
        "COUNT".bold(),
        "NEWEST".bold()
    );
    for row in &stats {
        println!(
            "{:<16} {:<8} {}",
            row.ecosystem.cyan(),
            row.count,
            row.newest.as_deref().unwrap_or("—")
        );
    }
    println!();
    println!(
        "{} {} cached verdict(s), newest checked at {} (UTC)",
        "Summary".bold(),
        total,
        newest.as_deref().unwrap_or("—")
    );
    Ok(())
}
