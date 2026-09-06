use std::path::{Path, PathBuf};

use colored::Colorize;

use crate::context::AppContext;
use crate::core::ball_parser::{parse_ball_file, BallManifest};
use crate::core::injected::{resolve_baller_dir, upsert_injected, InjectedCommand};
use crate::error::error::BallError;
use crate::utils::fs::confirm;

/// Names baller reserves for its own subcommands; an injected command may not
/// shadow any of them.
pub const RESERVED_COMMANDS: [&str; 11] = [
    "draft",
    "eject",
    "freeze",
    "roster",
    "substitute",
    "sweep",
    "update",
    "build",
    "inject",
    "help",
    "version",
];

/// Injects a command described by a `.ball` file into baller's runtime.
///
/// Injecting hands an arbitrary binary the ability to run under `baller <name>`,
/// so the user has to clear three separate confirmations first.
pub fn execute_inject(ctx: &AppContext, path: &str) -> Result<(), BallError> {
    if !ctx.flags.yes {
        if !confirm(
            "This will modify baller's runtime behavior by adding a new command. Continue? [yes/no]",
        ) {
            println!("Aborted.");
            return Ok(());
        }

        if !confirm(
            "Only inject .ball files from sources you trust: the binary they name runs with your privileges. Continue? [yes/no]",
        ) {
            println!("Aborted.");
            return Ok(());
        }
    }

    let manifest = parse_ball_file(Path::new(path))?;

    if !ctx.flags.yes
        && !confirm(&format!(
            "Final confirmation: inject '{}' from {}? [yes/no]",
            manifest.command_name.cyan(),
            manifest.path.display().to_string().cyan()
        ))
    {
        println!("Aborted.");
        return Ok(());
    }

    let binary_path = validate_manifest(&manifest)?;

    let baller_dir = resolve_baller_dir(&ctx.config);
    let mut command: InjectedCommand = manifest.into();
    command.path = binary_path;

    let name = command.command_name.clone();
    let binary = command.path.display().to_string();
    upsert_injected(&baller_dir, command)?;

    println!(
        "{} Injected '{}' -> {}",
        "Done".green().bold(),
        name.cyan(),
        binary
    );
    println!("Run it with: baller {}", name);

    Ok(())
}

/// True if `name` collides with one of baller's built-in subcommands.
pub fn is_reserved(name: &str) -> bool {
    RESERVED_COMMANDS.contains(&name.to_lowercase().as_str())
}

/// Rejects manifests baller cannot honor and returns the path to store.
///
/// The path is canonicalized so an injected command keeps working no matter
/// which directory it is later invoked from.
fn validate_manifest(manifest: &BallManifest) -> Result<PathBuf, BallError> {
    if is_reserved(&manifest.command_name) {
        return Err(BallError::InjectedCommandError(format!(
            "'{}' is a built-in baller command and cannot be injected",
            manifest.command_name
        )));
    }

    if manifest.command_name.split_whitespace().count() != 1 {
        return Err(BallError::InjectedCommandError(format!(
            "'{}' is not a valid command name: names cannot contain whitespace",
            manifest.command_name
        )));
    }

    static VALID_COMMAND_NAME: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^[A-Za-z0-9_][A-Za-z0-9_-]*$").unwrap());

    if !VALID_COMMAND_NAME.is_match(&manifest.command_name) {
        return Err(BallError::InjectedCommandError(format!(
            "'{}' is not a valid command name: must start with a letter, digit, or underscore \
                and contain only letters, digits, underscores, or hyphens",
            manifest.command_name
        )));
    }

    if !manifest.path.exists() {
        return Err(BallError::InjectedCommandError(format!(
            "no binary found at '{}' for command '{}'",
            manifest.path.display(),
            manifest.command_name
        )));
    }

    if !manifest.path.is_file() {
        return Err(BallError::InjectedCommandError(format!(
            "'{}' is not a file: expected an executable for command '{}'",
            manifest.path.display(),
            manifest.command_name
        )));
    }

    Ok(manifest
        .path
        .canonicalize()
        .unwrap_or_else(|_| manifest.path.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with(name: &str, path: PathBuf) -> BallManifest {
        BallManifest {
            command_name: name.to_string(),
            description: "desc".to_string(),
            version: "1.0.0".to_string(),
            flags: vec![],
            author: "HMythical".to_string(),
            require_root: false,
            depends: vec![],
            path,
        }
    }

    fn test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("baller_inject_tests")
            .join(format!("{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_all_builtins_are_reserved() {
        for name in RESERVED_COMMANDS {
            assert!(is_reserved(name), "{} should be reserved", name);
        }
    }

    #[test]
    fn test_reserved_check_is_case_insensitive() {
        assert!(is_reserved("Draft"));
        assert!(is_reserved("SWEEP"));
    }

    #[test]
    fn test_non_builtin_is_not_reserved() {
        assert!(!is_reserved("my-tool"));
        assert!(!is_reserved("drafter"));
    }

    #[test]
    fn test_validate_rejects_reserved_name() {
        let dir = test_dir("reserved");
        let binary = dir.join("draft");
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();

        let err = validate_manifest(&manifest_with("draft", binary)).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("built-in baller command"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_missing_binary() {
        let manifest = manifest_with("my-tool", PathBuf::from("/nonexistent/binary"));
        let err = validate_manifest(&manifest).unwrap_err();
        assert!(format!("{}", err).contains("no binary found"));
    }

    #[test]
    fn test_validate_rejects_directory_path() {
        let dir = test_dir("dir_path");
        let err = validate_manifest(&manifest_with("my-tool", dir.clone())).unwrap_err();
        assert!(format!("{}", err).contains("is not a file"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_name_with_whitespace() {
        let dir = test_dir("whitespace");
        let binary = dir.join("tool");
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();

        let err = validate_manifest(&manifest_with("my tool", binary)).unwrap_err();
        assert!(format!("{}", err).contains("valid command name"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_accepts_and_canonicalizes_existing_binary() {
        let dir = test_dir("canonicalize");
        let binary = dir.join("my-tool");
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();

        let relative = dir.join(".").join("my-tool");
        let resolved = validate_manifest(&manifest_with("my-tool", relative)).unwrap();
        assert_eq!(resolved, binary.canonicalize().unwrap());
        assert!(resolved.is_absolute());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_invalid_names() {
        macro_rules! test_invalid_name {
            ($name:literal, $dir:literal) => {
                let dir = test_dir($dir);
                let binary = dir.join("tool");
                std::fs::write(&binary, b"#!/bin/sh\n").unwrap();

                let err = validate_manifest(&manifest_with($name, binary)).unwrap_err();
                assert!(format!("{}", err).contains("valid command name"));

                let _ = std::fs::remove_dir_all(&dir);
            };
        }

        test_invalid_name!("--json", "name_with_dashes");
        test_invalid_name!("-j", "name_with_single_dash");
        test_invalid_name!("-", "lonely_dash");
        test_invalid_name!("$(rm -rf ~)", "dangerous_shell_subst");
        test_invalid_name!("foo;bar", "possible_command_sep");
        test_invalid_name!("foo|bar", "pipe");
        test_invalid_name!("foo&bar", "ampersand");
        test_invalid_name!("../inject", "path");
        test_invalid_name!("foo/bar", "path2");
        test_invalid_name!("café", "non_ascii");
        test_invalid_name!("foo\0bar", "null_byte");
        test_invalid_name!("", "empty");
    }

    #[test]
    fn test_validate_doesnt_reject_valid_names() {
        macro_rules! test_valid_name {
            ($name:literal, $dir:literal) => {
                let dir = test_dir($dir);
                let binary = dir.join("tool");
                std::fs::write(&binary, b"#!/bin/sh\n").unwrap();

                assert!(validate_manifest(&manifest_with($name, binary)).is_ok());
            };
        }

        test_valid_name!("foo", "usual_name");
        test_valid_name!("1foo", "number");
        test_valid_name!("foo1", "number_at_the_end");
        test_valid_name!("some_underscored_name", "underscored_name");
        test_valid_name!("name-with-dashes", "name_with_dashes");
    }
}
