//! `baller referee cache` — the verdict cache as a first-class surface.
//!
//! Works with Referee switched off: managing stored verdicts asks nothing of
//! the advisory service.

use colored::Colorize;
use serde_json::json;

use crate::context::AppContext;
use crate::core::db::RefereeCachePrune;
use crate::error::error::BallError;
use crate::utils::output::print_json;

/// What `baller referee cache` does. `Status` is the default.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CacheAction {
    Status,
    Clear,
    /// Drop verdicts computed more than `days` days ago. `vulnerable` ones
    /// are kept unless `include_vulnerable` is set.
    Prune {
        days: u32,
        include_vulnerable: bool,
    },
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
        CacheAction::Prune {
            days,
            include_vulnerable,
        } => {
            let outcome = ctx.db.referee_cache_prune(days, include_vulnerable)?;
            if ctx.flags.json {
                return print_json(&prune_json(days, &outcome));
            }
            println!(
                "{} {} cached verdict(s) older than {} day(s)",
                "Pruned".green().bold(),
                outcome.removed,
                days
            );
            // Reported at the default verbosity either way: what was kept is
            // what still blocks, and what was removed no longer does.
            if outcome.removed_vulnerable > 0 {
                println!(
                    "  {} {} of them were vulnerable verdicts — those packages will be re-queried",
                    "Note:".yellow(),
                    outcome.removed_vulnerable
                );
            }
            if outcome.kept_vulnerable > 0 {
                println!(
                    "  {} kept {} vulnerable verdict(s) older than {} day(s) — pass {} to remove them",
                    "Note:".yellow(),
                    outcome.kept_vulnerable,
                    days,
                    "--include-vulnerable".cyan()
                );
            }
            Ok(())
        }
    }
}

/// The `--prune` JSON document. The original five keys are unchanged; the two
/// vulnerable counts are additive.
fn prune_json(days: u32, outcome: &RefereeCachePrune) -> serde_json::Value {
    json!({
        "command": "referee",
        "subcommand": "cache",
        "action": "prune",
        "days": days,
        "removed": outcome.removed,
        "removed_vulnerable": outcome.removed_vulnerable,
        "kept_vulnerable": outcome.kept_vulnerable,
    })
}

fn status(ctx: &AppContext) -> Result<(), BallError> {
    let stats = ctx.db.referee_cache_stats()?;
    let total: i64 = stats.iter().map(|row| row.count).sum();
    // `checked_at` is `YYYY-MM-DD HH:MM:SS`, so the string maximum is the newest.
    let newest = stats.iter().filter_map(|row| row.newest.clone()).max();
    let ttl_days = ctx.config.referee.cache_ttl_days;
    let stale = ctx.db.referee_cache_stale_count(ttl_days)?;

    if ctx.flags.json {
        return print_json(&json!({
            "command": "referee",
            "subcommand": "cache",
            "action": "status",
            "total": total,
            "newest": newest,
            "cache_ttl_days": ttl_days,
            "stale": stale,
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
    println!("{} {}", "Freshness".bold(), freshness_note(ttl_days, stale));
    Ok(())
}

/// What the TTL means for the rows in the cache right now.
fn freshness_note(ttl_days: Option<u32>, stale: i64) -> String {
    match ttl_days {
        None => "cache_ttl_days is off, so cached verdicts never age out".to_string(),
        Some(days) => format!(
            "{} clean verdict(s) older than {} day(s) will be re-queried on the next install",
            stale, days
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_freshness_note_says_the_cache_never_ages_out_without_a_ttl() {
        assert!(freshness_note(None, 0).contains("never age out"));
        let note = freshness_note(Some(7), 3);
        assert!(note.starts_with("3 clean verdict(s) older than 7 day(s)"));
    }

    #[test]
    fn test_prune_json_keeps_every_key_and_adds_the_vulnerable_counts() {
        let doc = prune_json(
            30,
            &RefereeCachePrune {
                removed: 3,
                removed_vulnerable: 0,
                kept_vulnerable: 2,
            },
        );
        assert_eq!(doc["command"], "referee");
        assert_eq!(doc["subcommand"], "cache");
        assert_eq!(doc["action"], "prune");
        assert_eq!(doc["days"], 30);
        assert_eq!(doc["removed"], 3);
        assert_eq!(doc["removed_vulnerable"], 0);
        assert_eq!(doc["kept_vulnerable"], 2);
        assert_eq!(doc.as_object().unwrap().len(), 7);
    }
}
