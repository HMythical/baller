use serde::Serialize;

use crate::error::error::BallError;

/// Print a value as pretty JSON on stdout.
///
/// Used by commands running under the global `--json` flag, where every other
/// print must be suppressed so stdout stays machine-readable.
pub fn print_json<T: Serialize>(value: &T) -> Result<(), BallError> {
    let rendered = serde_json::to_string_pretty(value)
        .map_err(|e| BallError::InvalidConfig(format!("failed to render JSON output: {}", e)))?;
    println!("{}", rendered);
    Ok(())
}

/// Emit a progress/status line unless output is suppressed.
///
/// Goes out through `tracing` at `INFO`, so it lands on stderr and stdout is
/// left to the data a command produces.
pub fn info(quiet: bool, message: impl AsRef<str>) {
    if !quiet {
        tracing::info!("{}", message.as_ref());
    }
}

/// Emit verbose detail, seen only when `--verbose` raises the tracer to `DEBUG`.
///
/// No `quiet` argument: `--quiet` and `--json` already pin the tracer to
/// `ERROR`, which drops these events before they are formatted.
pub fn debug(message: impl AsRef<str>) {
    tracing::debug!("{}", message.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_print_json_serializes() {
        let value = json!({ "command": "roster", "count": 2 });
        assert!(print_json(&value).is_ok());
    }

    #[test]
    fn test_info_is_quiet_safe() {
        info(true, "suppressed");
        info(false, "printed");
    }

    #[test]
    fn test_debug_without_subscriber_is_safe() {
        debug("verbose detail");
        debug(format!("formatted {}", "detail"));
    }
}
