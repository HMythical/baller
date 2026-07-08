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

    /// Parse TOML manifest content
    ///
    /// # Arguments
    /// * `content` - TOML manifest content as string
    ///
    /// # Returns
    /// * `Result<Package, BallError>` - Parsed package or error
    pub fn parse_toml(content: &str) -> Result<Package, BallError> {
        toml::from_str(content)
            .map_err(|e| BallError::InvalidConfig(format!("Failed to parse TOML manifest: {}", e)))
    }

    /// Parse JSON manifest content
    ///
    /// # Arguments
    /// * `content` - JSON manifest content as string
    ///
    /// # Returns
    /// * `Result<Package, BallError>` - Parsed package or error
    pub fn parse_json(content: &str) -> Result<Package, BallError> {
        serde_json::from_str(content)
            .map_err(|e| BallError::InvalidConfig(format!("Failed to parse JSON manifest: {}", e)))
    }

    /// Parse manifest from file with automatic format detection
    ///
    /// # Arguments
    /// * `path` - Path to manifest file
    ///
    /// # Returns
    /// * `Result<Package, BallError>` - Parsed package or error
    ///
    /// # Formats Supported
    /// - TOML: file with .toml extension
    /// - JSON: file with .json extension
    /// - Default: Any file (assumed TOML format for backwards compatibility)
    pub fn parse_auto(path: &PathBuf) -> Result<Package, BallError> {
        match Self::parse(path) {
            Ok(pkg) => Ok(pkg),
            Err(BallError::FileIoErr(_)) => Err(BallError::InvalidConfig(format!(
                "Manifest file not found: {}",
                path.display()
            ))),
            Err(e) => Err(e),
        }
    }

    /// Validate manifest requirements (name and version are required fields)
    ///
    /// # Arguments
    /// * `package` - Package to validate
    ///
    /// # Returns
    /// * `Result<(), BallError>` - Success if valid, error otherwise
    pub fn validate(package: &Package) -> Result<(), BallError> {
        if package.name.is_empty() {
            return Err(BallError::InvalidConfig(
                "Package name is required".to_string(),
            ));
        }
        if package.version.is_empty() {
            return Err(BallError::InvalidConfig(
                "Package version is required".to_string(),
            ));
        }
        Ok(())
    }

    /// Create example manifest from package
    ///
    /// # Arguments
    /// * `package` - Package to convert
    /// * `format` - Output format ("toml" or "json")
    ///
    /// # Returns
    /// * `Result<String, BallError>` - Serialized manifest
    pub fn serialize(package: &Package, format: &str) -> Result<String, BallError> {
        match format.to_lowercase().as_str() {
            "json" => serde_json::to_string(package).map_err(|e| {
                BallError::InvalidConfig(format!("Failed to serialize to JSON: {}", e))
            }),
            "toml" => toml::to_string(package).map_err(|e| {
                BallError::InvalidConfig(format!("Failed to serialize to TOML: {}", e))
            }),
            _ => Err(BallError::InvalidConfig(format!(
                "Unsupported format: {}",
                format
            ))),
        }
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
