use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde_json::{json, Value};

use crate::error::error::BallError;

/// Set once a document has gone to stdout, so the error path in `main` can
/// tell whether stdout is still free for a JSON error.
static DOCUMENT_PRINTED: AtomicBool = AtomicBool::new(false);

/// Print a value as pretty JSON on stdout.
///
/// Used by commands running under the global `--json` flag, where every other
/// print must be suppressed so stdout stays machine-readable.
pub fn print_json<T: Serialize>(value: &T) -> Result<(), BallError> {
    let rendered = serde_json::to_string_pretty(value)
        .map_err(|e| BallError::InvalidConfig(format!("failed to render JSON output: {}", e)))?;
    println!("{}", rendered);
    mark_document_printed();
    Ok(())
}

/// Record that this run has put its document on stdout.
///
/// `print_json` calls this itself; anything else that writes a whole document
/// to stdout (`referee audit --format`, `referee sbom`) calls it directly.
pub fn mark_document_printed() {
    DOCUMENT_PRINTED.store(true, Ordering::SeqCst);
}

/// Whether a document has already gone to stdout during this run.
///
/// `referee --fail-on` prints its report and then fails; a second document
/// after it would leave stdout unparseable, so its error stays on stderr.
pub fn document_printed() -> bool {
    DOCUMENT_PRINTED.load(Ordering::SeqCst)
}

/// The document a failed `--json` run prints in place of `[Error]: …`.
///
/// `command` is the subcommand that failed, or `None` when the command line
/// could not be parsed far enough to name one.
pub fn error_json(command: Option<&str>, err: &BallError) -> Value {
    json!({
        "command": command,
        "error": err.to_string(),
        "code": err.code(),
    })
}

/// Print `err` as the run's JSON document on stdout.
pub fn print_json_error(command: Option<&str>, err: &BallError) -> Result<(), BallError> {
    print_json(&error_json(command, err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_print_json_serializes() {
        let value = json!({ "command": "roster", "count": 2 });
        assert!(print_json(&value).is_ok());
        assert!(document_printed());
    }

    #[test]
    fn test_error_json_carries_command_message_and_code() {
        let err = BallError::PackageNotFound("fd".to_string());
        let value = error_json(Some("draft"), &err);
        assert_eq!(value["command"], "draft");
        assert_eq!(value["error"], err.to_string());
        assert_eq!(value["code"], "PackageNotFound");
    }

    #[test]
    fn test_error_json_without_a_command_is_null() {
        let err = BallError::InvalidConfig("CLI Error: bad".to_string());
        let value = error_json(None, &err);
        assert!(value["command"].is_null());
        assert_eq!(value["code"], "InvalidConfig");
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
