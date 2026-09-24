//! `baller referee config` — the `[referee]` settings actually in effect.
//!
//! The VirusTotal key is reported as set or unset; its value is never printed.

use colored::Colorize;
use serde_json::{json, Value};

use crate::context::AppContext;
use crate::error::error::BallError;
use crate::utils::output::print_json;

pub fn execute_config(ctx: &AppContext) -> Result<(), BallError> {
    let settings = effective_settings(ctx);

    if ctx.flags.json {
        return print_json(&json!({
            "command": "referee",
            "subcommand": "config",
            "config": settings,
        }));
    }

    let rows = [
        ("enabled", settings["enabled"].to_string()),
        ("warn_at", format!("{:.2}", ctx.config.referee.warn_at)),
        ("block_at", format!("{:.2}", ctx.config.referee.block_at)),
        (
            "fail_policy",
            ctx.config.referee.fail_policy.label().to_string(),
        ),
        ("osv_base_url", ctx.config.referee.osv_base_url.clone()),
        (
            "virustotal_base_url",
            ctx.config
                .referee
                .virustotal_base_url
                .clone()
                .unwrap_or_else(|| "<default>".to_string()),
        ),
        (
            "virustotal_api_key",
            key_state(ctx.config.referee.virustotal_api_key.as_deref()).to_string(),
        ),
    ];

    println!("{}", "[referee]".bold());
    for (key, value) in rows {
        println!("{:<20} = {}", key.cyan(), value);
    }

    if ctx.config.referee.enabled && !ctx.referee.enabled() {
        println!();
        println!(
            "{} enabled in baller.conf, but switched off for this run by --no-referee",
            "Note".yellow()
        );
    }
    Ok(())
}

/// The effective settings. `enabled` accounts for `--no-referee`.
fn effective_settings(ctx: &AppContext) -> Value {
    let referee = &ctx.config.referee;
    json!({
        "enabled": ctx.referee.enabled(),
        "warn_at": referee.warn_at,
        "block_at": referee.block_at,
        "fail_policy": referee.fail_policy.label(),
        "osv_base_url": referee.osv_base_url,
        "virustotal_base_url": referee.virustotal_base_url,
        "virustotal_api_key": key_state(referee.virustotal_api_key.as_deref()),
    })
}

/// Whether a key is configured, without ever revealing it.
fn key_state(key: Option<&str>) -> &'static str {
    match key {
        Some(key) if !key.trim().is_empty() => "set",
        _ => "unset",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_state_never_echoes_the_key() {
        assert_eq!(key_state(Some("secret-key-123")), "set");
        assert_eq!(key_state(Some("   ")), "unset");
        assert_eq!(key_state(None), "unset");
    }
}
