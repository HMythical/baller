use crate::core::package::Package;
use crate::error::error::BallError;
use std::fs;
use std::path::PathBuf;

#[allow(dead_code)]
pub struct ManifestParser;

#[allow(dead_code)]
impl ManifestParser {
    /// Loads a package manifest locally, guessing the format via extension.
    /// Defaults to TOML.
    pub fn parse(path: &PathBuf) -> Result<Package, BallError> {
        let content = fs::read_to_string(path).map_err(BallError::FileIoErr)?;

        let path_str = path.display().to_string();
        if path_str.ends_with(".toml") {
            Self::parse_toml(&content)
        } else if path_str.ends_with(".json") {
            Self::parse_json(&content)
        } else {
            Self::parse_toml(&content) // Standard Format
        }
    }

    pub fn parse_toml(content: &str) -> Result<Package, BallError> {
        toml::from_str(content)
            .map_err(|e| BallError::InvalidConfig(format!("Failed to parse TOML manifest: {}", e)))
    }

    pub fn parse_json(content: &str) -> Result<Package, BallError> {
        serde_json::from_str(content)
            .map_err(|e| BallError::InvalidConfig(format!("Failed to parse JSON manifest: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_toml_valid() {
        let toml = r#"
name = "test-pkg"
version = "1.0.0"
description = "A test package"
author = "Test Author"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        assert_eq!(pkg.name, "test-pkg");
        assert_eq!(pkg.version, "1.0.0");
        assert_eq!(pkg.description.unwrap(), "A test package");
        assert_eq!(pkg.author.unwrap(), "Test Author");
    }

    #[test]
    fn test_parse_toml_with_all_fields() {
        let toml = r#"
name = "full-pkg"
version = "2.0.0"
description = "Full package"
author = "Author"
repository = "https://github.com/user/repo"
dependencies = ["dep1", "dep2 >=1.0"]
sha256 = "abc123"
download_url = "https://example.com/pkg.tar.gz"

[source]
GitHub = { owner = "owner", repo = "repo" }
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        assert_eq!(pkg.name, "full-pkg");
        assert_eq!(pkg.version, "2.0.0");
        assert!(pkg.dependencies.is_some());
        assert_eq!(pkg.dependencies.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_parse_toml_invalid() {
        let toml = "not valid toml {{{";
        let result = ManifestParser::parse_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_toml_missing_required() {
        let toml = r#"
description = "missing name and version"
"#;
        let result = ManifestParser::parse_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_json_valid() {
        let json = r#"{
    "name": "json-pkg",
    "version": "3.0.0",
    "description": "A JSON package",
    "author": "JSON Author"
}"#;
        let pkg = ManifestParser::parse_json(json).unwrap();
        assert_eq!(pkg.name, "json-pkg");
        assert_eq!(pkg.version, "3.0.0");
        assert_eq!(pkg.description.unwrap(), "A JSON package");
    }

    #[test]
    fn test_parse_json_invalid() {
        let json = "{ invalid json }";
        let result = ManifestParser::parse_json(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_json_empty() {
        let result = ManifestParser::parse_json("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_guess_format_toml() {
        let dir = std::env::temp_dir().join("baller_test_manifest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("baller.toml");
        std::fs::write(
            &path,
            r#"name = "guess-pkg"
version = "1.0.0""#,
        )
        .unwrap();

        let pkg = ManifestParser::parse(&path).unwrap();
        assert_eq!(pkg.name, "guess-pkg");
        assert_eq!(pkg.version, "1.0.0");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_guess_format_json() {
        let dir = std::env::temp_dir().join("baller_test_manifest_json");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("baller.json");
        std::fs::write(&path, r#"{"name": "json-pkg", "version": "2.0.0"}"#).unwrap();

        let pkg = ManifestParser::parse(&path).unwrap();
        assert_eq!(pkg.name, "json-pkg");
        assert_eq!(pkg.version, "2.0.0");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_file_not_found() {
        let path = PathBuf::from("/nonexistent/manifest.toml");
        let result = ManifestParser::parse(&path);
        assert!(result.is_err());
    }
}
