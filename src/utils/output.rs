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
        tracing::info!(true, "suppressed");
        tracing::info!(false, "printed");
    }

    #[test]
    fn test_debug_without_subscriber_is_safe() {
        tracing::debug!("verbose detail");
        tracing::debug!("formatted {}", "detail");
    }
}
