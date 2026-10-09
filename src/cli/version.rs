use serde_json::json;

use crate::context::GlobalFlags;
use crate::error::error::BallError;
use crate::utils::output::print_json;

pub fn execute_command_version(flags: &GlobalFlags) -> Result<(), BallError> {
    if flags.json {
        return print_json(&json!({
            "command": "version",
            "version": env!("CARGO_PKG_VERSION"),
        }));
    }

    println!("Baller {}", env!("CARGO_PKG_VERSION"));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_runs_in_text_and_json() {
        assert!(execute_command_version(&GlobalFlags::default()).is_ok());
        let flags = GlobalFlags {
            json: true,
            ..GlobalFlags::default()
        };
        assert!(execute_command_version(&flags).is_ok());
    }
}
