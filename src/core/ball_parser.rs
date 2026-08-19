use std::fs;
use std::path::{Path, PathBuf};

use crate::error::error::BallError;

/// Version stamped on a manifest that omits the `[VERSION]` section.
const DEFAULT_VERSION: &str = "0.1.0";

const KNOWN_SECTIONS: [&str; 8] = [
    "COMMAND-NAME",
    "DESCRIPTION",
    "VERSION",
    "FLAGS",
    "AUTHOR",
    "REQUIRE-ROOT",
    "DEPENDS",
    "PATH",
];

/// A parsed `.ball` file: everything baller needs to run an injected command.
#[derive(Debug, Clone, PartialEq)]
pub struct BallManifest {
    pub command_name: String,
    pub description: String,
    pub version: String,
    pub flags: Vec<String>,
    pub author: String,
    pub require_root: bool,
    pub depends: Vec<String>,
    pub path: PathBuf,
}

/// Reads and parses a `.ball` file from disk.
pub fn parse_ball_file(path: &Path) -> Result<BallManifest, BallError> {
    let content = fs::read_to_string(path).map_err(BallError::FileIoErr)?;
    parse_ball(&content)
}

/// Parses `.ball` content: `[SECTION]` headers with `KEY = VALUE` entries.
///
/// `COMMAND-NAME` and `PATH` are required; every other section is optional.
pub fn parse_ball(content: &str) -> Result<BallManifest, BallError> {
    let mut command_name: Option<String> = None;
    let mut description = String::new();
    let mut version: Option<String> = None;
    let mut flags: Vec<String> = Vec::new();
    let mut author = String::new();
    let mut require_root = false;
    let mut depends: Vec<String> = Vec::new();
    let mut path: Option<PathBuf> = None;

    for (index, raw_line) in content.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw_line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = trimmed[1..trimmed.len() - 1].trim().to_uppercase();
            if !KNOWN_SECTIONS.contains(&section.as_str()) {
                return Err(BallError::InvalidConfig(format!(
                    "unknown .ball section at line[{}]: '[{}]'",
                    line, section
                )));
            }
            continue;
        }

        let eq_pos = trimmed.find('=').ok_or_else(|| {
            BallError::InvalidConfig(format!(
                "invalid .ball entry at line[{}]: expected 'KEY = VALUE'",
                line
            ))
        })?;

        let key = trimmed[..eq_pos].trim().to_uppercase();
        let value = unquote(&trimmed[eq_pos + 1..]);

        if key.is_empty() {
            return Err(BallError::InvalidConfig(format!(
                "invalid .ball entry at line[{}]: empty key",
                line
            )));
        }

        match key.as_str() {
            "COMMAND-NAME" => {
                require_value(&value, "COMMAND-NAME", line)?;
                command_name = Some(value);
            }
            "DESCRIPTION" => description = value,
            "VERSION" => {
                require_value(&value, "VERSION", line)?;
                version = Some(value);
            }
            "FLAGS-LIST" => flags = split_list(&value),
            "AUTHOR" => author = value,
            "ROOTPERMS" => require_root = parse_bool(&value, line)?,
            "DEPENDS" => depends = split_list(&value),
            "PATH" => {
                require_value(&value, "PATH", line)?;
                path = Some(PathBuf::from(value));
            }
            _ => {
                return Err(BallError::InvalidConfig(format!(
                    "unknown .ball key at line[{}]: '{}'",
                    line, key
                )));
            }
        }
    }

    let command_name = command_name.ok_or_else(|| {
        BallError::InvalidConfig(
            "invalid .ball file: missing required section '[COMMAND-NAME]'".to_string(),
        )
    })?;

    let path = path.ok_or_else(|| {
        BallError::InvalidConfig(
            "invalid .ball file: missing required section '[PATH]'".to_string(),
        )
    })?;

    Ok(BallManifest {
        command_name,
        description,
        version: version.unwrap_or_else(|| DEFAULT_VERSION.to_string()),
        flags,
        author,
        require_root,
        depends,
        path,
    })
}

/// Trims whitespace and a single layer of matching quotes from a value.
fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();

    if bytes.len() >= 2 {
        let first = bytes[0] as char;
        let last = bytes[bytes.len() - 1] as char;
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            return trimmed[1..trimmed.len() - 1].to_string();
        }
    }

    trimmed.to_string()
}

/// Splits a comma separated value, dropping empty entries.
///
/// Accepts both documented styles: one quoted list (`"-y, --yes"`) and
/// individually quoted entries (`"-y", "--yes"`).
fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|entry| {
            entry
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn require_value(value: &str, key: &str, line: usize) -> Result<(), BallError> {
    if value.is_empty() {
        return Err(BallError::InvalidConfig(format!(
            "invalid .ball entry at line[{}]: '{}' cannot be empty",
            line, key
        )));
    }
    Ok(())
}

fn parse_bool(value: &str, line: usize) -> Result<bool, BallError> {
    match value.to_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        _ => Err(BallError::InvalidConfig(format!(
            "invalid boolean '{}' at line[{}]: expected true/false, yes/no, 1/0, or on/off",
            value, line
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_BALL: &str = r#"
[COMMAND-NAME]
COMMAND-NAME = "my-tool"

[DESCRIPTION]
DESCRIPTION = "A helpful tool that does X"

[VERSION]
VERSION = "1.0.0"

[FLAGS]
FLAGS-LIST = "-y, --yes, -n, --no"

[AUTHOR]
AUTHOR = "HMythical"

[REQUIRE-ROOT]
ROOTPERMS = false

[DEPENDS]
DEPENDS = "python3, ffmpeg"

[PATH]
PATH = /usr/local/bin/my-tool
"#;

    #[test]
    fn test_parse_full_manifest() {
        let manifest = parse_ball(FULL_BALL).unwrap();
        assert_eq!(manifest.command_name, "my-tool");
        assert_eq!(manifest.description, "A helpful tool that does X");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.flags, vec!["-y", "--yes", "-n", "--no"]);
        assert_eq!(manifest.author, "HMythical");
        assert!(!manifest.require_root);
        assert_eq!(manifest.depends, vec!["python3", "ffmpeg"]);
        assert_eq!(manifest.path, PathBuf::from("/usr/local/bin/my-tool"));
    }

    #[test]
    fn test_parse_minimal_manifest_uses_defaults() {
        let content = "[COMMAND-NAME]\nCOMMAND-NAME = tool\n[PATH]\nPATH = /bin/tool\n";
        let manifest = parse_ball(content).unwrap();
        assert_eq!(manifest.command_name, "tool");
        assert_eq!(manifest.version, DEFAULT_VERSION);
        assert_eq!(manifest.description, "");
        assert_eq!(manifest.author, "");
        assert!(!manifest.require_root);
        assert!(manifest.flags.is_empty());
        assert!(manifest.depends.is_empty());
    }

    #[test]
    fn test_parse_missing_command_name() {
        let content = "[PATH]\nPATH = /bin/tool\n";
        let err = parse_ball(content).unwrap_err();
        assert!(format!("{}", err).contains("[COMMAND-NAME]"));
    }

    #[test]
    fn test_parse_missing_path() {
        let content = "[COMMAND-NAME]\nCOMMAND-NAME = tool\n";
        let err = parse_ball(content).unwrap_err();
        assert!(format!("{}", err).contains("[PATH]"));
    }

    #[test]
    fn test_parse_malformed_line_without_equals() {
        let content = "[FLAGS]\nFLAGS-LIST + \"-y\", \"--yes\"\n";
        let err = parse_ball(content).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("line[2]"));
        assert!(msg.contains("KEY = VALUE"));
    }

    #[test]
    fn test_parse_unknown_section() {
        let content = "[NOT-A-SECTION]\nfoo = bar\n";
        let err = parse_ball(content).unwrap_err();
        assert!(format!("{}", err).contains("unknown .ball section"));
    }

    #[test]
    fn test_parse_unknown_key() {
        let content = "[COMMAND-NAME]\nBOGUS = value\n";
        let err = parse_ball(content).unwrap_err();
        assert!(format!("{}", err).contains("unknown .ball key"));
    }

    #[test]
    fn test_parse_empty_key() {
        let content = " = value\n";
        let err = parse_ball(content).unwrap_err();
        assert!(format!("{}", err).contains("empty key"));
    }

    #[test]
    fn test_parse_empty_required_value() {
        let content = "[COMMAND-NAME]\nCOMMAND-NAME = \"\"\n[PATH]\nPATH = /bin/tool\n";
        let err = parse_ball(content).unwrap_err();
        assert!(format!("{}", err).contains("cannot be empty"));
    }

    #[test]
    fn test_parse_comments_and_blank_lines_ignored() {
        let content = "# a comment\n\n; another comment\n[COMMAND-NAME]\nCOMMAND-NAME = tool\n[PATH]\nPATH = /bin/tool\n";
        let manifest = parse_ball(content).unwrap();
        assert_eq!(manifest.command_name, "tool");
    }

    #[test]
    fn test_parse_keys_are_case_insensitive() {
        let content = "[command-name]\ncommand-name = tool\n[path]\npath = /bin/tool\n";
        let manifest = parse_ball(content).unwrap();
        assert_eq!(manifest.command_name, "tool");
        assert_eq!(manifest.path, PathBuf::from("/bin/tool"));
    }

    #[test]
    fn test_parse_root_perms_variants() {
        for (value, expected) in [
            ("true", true),
            ("YES", true),
            ("1", true),
            ("on", true),
            ("false", false),
            ("no", false),
            ("0", false),
            ("OFF", false),
        ] {
            let content = format!(
                "[COMMAND-NAME]\nCOMMAND-NAME = tool\n[REQUIRE-ROOT]\nROOTPERMS = {}\n[PATH]\nPATH = /bin/tool\n",
                value
            );
            let manifest = parse_ball(&content).unwrap();
            assert_eq!(manifest.require_root, expected, "ROOTPERMS = {}", value);
        }
    }

    #[test]
    fn test_parse_invalid_root_perms() {
        let content = "[COMMAND-NAME]\nCOMMAND-NAME = tool\n[REQUIRE-ROOT]\nROOTPERMS = maybe\n[PATH]\nPATH = /bin/tool\n";
        let err = parse_ball(content).unwrap_err();
        assert!(format!("{}", err).contains("invalid boolean"));
    }

    #[test]
    fn test_parse_flags_with_individually_quoted_entries() {
        let content = "[COMMAND-NAME]\nCOMMAND-NAME = tool\n[FLAGS]\nFLAGS-LIST = \"-y\", \"--yes\"\n[PATH]\nPATH = /bin/tool\n";
        let manifest = parse_ball(content).unwrap();
        assert_eq!(manifest.flags, vec!["-y", "--yes"]);
    }

    #[test]
    fn test_parse_empty_optional_lists() {
        let content = "[COMMAND-NAME]\nCOMMAND-NAME = tool\n[DEPENDS]\nDEPENDS = \"\"\n[PATH]\nPATH = /bin/tool\n";
        let manifest = parse_ball(content).unwrap();
        assert!(manifest.depends.is_empty());
    }

    #[test]
    fn test_parse_value_containing_equals() {
        let content = "[COMMAND-NAME]\nCOMMAND-NAME = tool\n[DESCRIPTION]\nDESCRIPTION = \"key=value pairs\"\n[PATH]\nPATH = /bin/tool\n";
        let manifest = parse_ball(content).unwrap();
        assert_eq!(manifest.description, "key=value pairs");
    }

    #[test]
    fn test_unquote_helpers() {
        assert_eq!(unquote("  \"quoted\"  "), "quoted");
        assert_eq!(unquote("'single'"), "single");
        assert_eq!(unquote("bare"), "bare");
        assert_eq!(unquote("\"unbalanced"), "\"unbalanced");
    }

    #[test]
    fn test_parse_ball_file_reads_from_disk() {
        let dir = std::env::temp_dir().join(format!("baller_ball_parser_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("valid.ball");
        std::fs::write(&file, FULL_BALL).unwrap();

        let manifest = parse_ball_file(&file).unwrap();
        assert_eq!(manifest.command_name, "my-tool");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_ball_file_missing_file() {
        let missing = PathBuf::from("/nonexistent/path/to/file.ball");
        assert!(parse_ball_file(&missing).is_err());
    }
}
