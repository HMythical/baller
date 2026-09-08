use std::io::stderr;

use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

/// Environment variable that overrides the level computed from the CLI flags.
///
/// Deliberately not `RUST_LOG`: baller's tracer is user-facing output, not
/// library diagnostics, so it should not react to an env var meant for crates.
const LOG_ENV: &str = "BALLER_LOG";

/// The only target the tracer reports on by default.
///
/// Baller's dependency tree (reqwest, hyper, rustls) is instrumented too, and
/// `-v` is meant to explain what baller did — not to dump TLS handshakes.
const CRATE_TARGET: &str = env!("CARGO_CRATE_NAME");

/// The tracer's maximum level, as decided by the global CLI flags.
///
/// `--quiet` and `--json` silence everything below `ERROR`: `--json` in
/// particular must leave stdout holding nothing but the JSON document.
pub fn level_for(verbose: bool, quiet: bool, json: bool) -> LevelFilter {
    if quiet || json {
        LevelFilter::ERROR
    } else if verbose {
        LevelFilter::DEBUG
    } else {
        LevelFilter::INFO
    }
}

/// Install the global tracing subscriber for this run.
///
/// Everything the tracer emits goes to stderr; stdout stays reserved for data
/// (JSON documents and command results). Re-initialization is a no-op, so tests
/// that call this more than once do not panic.
pub fn init_tracing(verbose: bool, quiet: bool, json: bool, color: bool) {
    let level = level_for(verbose, quiet, json);

    let builder = tracing_subscriber::fmt()
        .with_level(false)
        .with_env_filter(env_filter(level, quiet || json))
        .with_writer(stderr)
        .with_ansi(color)
        .with_ansi_sanitization(false)
        .without_time()
        .with_target(false);

    // Errors here only mean a subscriber is already installed.
    let _ = builder.try_init();
}

/// The computed level, unless the user asked for something else via
/// `BALLER_LOG`. A silenced tracer (`--quiet`/`--json`) ignores the variable so
/// machine-readable runs can never be polluted from the environment.
fn env_filter(level: LevelFilter, silenced: bool) -> EnvFilter {
    let default = crate_directive(level);

    if silenced {
        return EnvFilter::new(default);
    }

    EnvFilter::builder()
        .with_default_directive(default.parse().unwrap_or_else(|_| LevelFilter::INFO.into()))
        .with_env_var(LOG_ENV)
        .from_env_lossy()
}

/// `<crate>=<level>`: the level applies to baller's own events only.
fn crate_directive(level: LevelFilter) -> String {
    format!("{}={}", CRATE_TARGET, level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verbose_maps_to_debug() {
        assert_eq!(level_for(true, false, false), LevelFilter::DEBUG);
    }

    #[test]
    fn test_default_maps_to_info() {
        assert_eq!(level_for(false, false, false), LevelFilter::INFO);
    }

    #[test]
    fn test_quiet_maps_to_error() {
        assert_eq!(level_for(false, true, false), LevelFilter::ERROR);
    }

    #[test]
    fn test_json_maps_to_error() {
        assert_eq!(level_for(false, false, true), LevelFilter::ERROR);
    }

    #[test]
    fn test_quiet_and_json_outrank_verbose() {
        assert_eq!(level_for(true, true, false), LevelFilter::ERROR);
        assert_eq!(level_for(true, false, true), LevelFilter::ERROR);
    }

    #[test]
    fn test_directive_is_scoped_to_this_crate() {
        assert_eq!(crate_directive(LevelFilter::DEBUG), "baller=debug");
    }

    #[test]
    fn test_silenced_filter_ignores_env_override() {
        let filter = env_filter(LevelFilter::ERROR, true);
        assert_eq!(filter.max_level_hint(), Some(LevelFilter::ERROR));
    }

    #[test]
    fn test_init_tracing_is_safe_to_call_twice() {
        init_tracing(false, true, false, false);
        init_tracing(true, false, false, false);
    }
}
