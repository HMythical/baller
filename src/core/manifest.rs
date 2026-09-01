use crate::core::package::Package;
use crate::error::error::BallError;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

const DEFAULT_CHOCOLATEY_FEED: &str = "https://community.chocolatey.org/api/v2";

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
    /// Accepts both the flat layout (`sha256`, `dependencies = [...]`,
    /// `[source] GitHub = { .. }`) and the nested layout documented in
    /// `docs/manifest.md` (`[source] type = ".."`, `[checksum]`,
    /// `[architectures]`, `[dependencies]` as a table).
    ///
    /// # Arguments
    /// * `content` - TOML manifest content as string
    ///
    /// # Returns
    /// * `Result<Package, BallError>` - Parsed package or error
    pub fn parse_toml(content: &str) -> Result<Package, BallError> {
        let raw: toml::Value = toml::from_str(content).map_err(|e| {
            BallError::InvalidConfig(format!("Failed to parse TOML manifest: {}", e))
        })?;
        let value = serde_json::to_value(raw).map_err(|e| {
            BallError::InvalidConfig(format!("Failed to parse TOML manifest: {}", e))
        })?;
        Self::from_value(value, "TOML")
    }

    /// Parse JSON manifest content
    ///
    /// Accepts both the flat and the nested layouts, exactly like
    /// [`ManifestParser::parse_toml`].
    ///
    /// # Arguments
    /// * `content` - JSON manifest content as string
    ///
    /// # Returns
    /// * `Result<Package, BallError>` - Parsed package or error
    pub fn parse_json(content: &str) -> Result<Package, BallError> {
        let value: Value = serde_json::from_str(content).map_err(|e| {
            BallError::InvalidConfig(format!("Failed to parse JSON manifest: {}", e))
        })?;
        Self::from_value(value, "JSON")
    }

    /// Normalize a decoded manifest into the flat `Package` shape
    fn from_value(value: Value, format: &str) -> Result<Package, BallError> {
        let normalized = normalize_manifest(value)?;
        serde_json::from_value(normalized).map_err(|e| {
            BallError::InvalidConfig(format!("Failed to parse {} manifest: {}", format, e))
        })
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

/// Rewrite the documented nested manifest layout into the flat `Package` shape.
///
/// Manifests already written in the flat layout pass through untouched.
fn normalize_manifest(value: Value) -> Result<Value, BallError> {
    let mut map = match value {
        Value::Object(map) => map,
        _ => {
            return Err(BallError::InvalidConfig(
                "manifest must be a table of key/value pairs".to_string(),
            ))
        }
    };

    normalize_checksum(&mut map);
    normalize_architectures(&mut map);
    normalize_dependencies(&mut map)?;
    normalize_source(&mut map)?;

    Ok(Value::Object(map))
}

/// `[checksum] sha256 = ".."` becomes the top-level `sha256` field
fn normalize_checksum(map: &mut Map<String, Value>) {
    let checksum = match map.get("checksum") {
        Some(Value::Object(table)) => table.clone(),
        _ => return,
    };

    if !map.contains_key("sha256") {
        if let Some(hash @ Value::String(_)) = checksum.get("sha256") {
            map.insert("sha256".to_string(), hash.clone());
        }
    }

    if !map.contains_key("hash_algorithm") {
        let algorithm = checksum
            .get("algorithm")
            .or_else(|| checksum.get("hash_algorithm"));
        if let Some(algorithm @ Value::String(_)) = algorithm {
            map.insert("hash_algorithm".to_string(), algorithm.clone());
        }
    }

    map.remove("checksum");
}

/// `[architectures] supported = [..]` becomes the top-level `architectures` list
fn normalize_architectures(map: &mut Map<String, Value>) {
    let supported = match map.get("architectures") {
        Some(Value::Object(table)) => table.get("supported").cloned(),
        _ => return,
    };

    match supported {
        Some(list @ Value::Array(_)) => {
            map.insert("architectures".to_string(), list);
        }
        _ => {
            map.remove("architectures");
        }
    }
}

/// A `[dependencies]` table of name → constraint becomes the flat string list.
/// The `?` optional prefix stays on the name, matching the array form.
fn normalize_dependencies(map: &mut Map<String, Value>) -> Result<(), BallError> {
    let table = match map.get("dependencies") {
        Some(Value::Object(table)) => table.clone(),
        _ => return Ok(()),
    };

    let mut dependencies = Vec::new();
    for (name, constraint) in table {
        let name = name.trim();
        if name.is_empty() {
            return Err(BallError::InvalidConfig(
                "dependency name cannot be empty".to_string(),
            ));
        }

        let entry = match constraint {
            Value::String(constraint) => {
                let constraint = constraint.trim();
                if constraint.is_empty() || constraint == "*" {
                    name.to_string()
                } else {
                    format!("{} {}", name, constraint)
                }
            }
            Value::Null => name.to_string(),
            _ => {
                return Err(BallError::InvalidConfig(format!(
                    "dependency '{}' must map to a version constraint string",
                    name
                )))
            }
        };

        dependencies.push(Value::String(entry));
    }

    map.insert("dependencies".to_string(), Value::Array(dependencies));
    Ok(())
}

/// `[source] type = ".."` becomes the tagged `PackageSource` representation
fn normalize_source(map: &mut Map<String, Value>) -> Result<(), BallError> {
    let source = match map.get("source") {
        Some(Value::Object(table)) => table.clone(),
        Some(_) => {
            return Err(BallError::InvalidConfig(
                "manifest 'source' must be a table".to_string(),
            ))
        }
        None => return Ok(()),
    };

    let source_type = match source.get("type") {
        Some(Value::String(source_type)) => source_type.trim().to_lowercase(),
        Some(_) => {
            return Err(BallError::InvalidConfig(
                "source 'type' must be a string".to_string(),
            ))
        }
        None => return Ok(()),
    };

    let normalized = match source_type.as_str() {
        "github" => {
            let (owner, repo) = github_owner_repo(&source, map)?;
            json!({ "GitHub": { "owner": owner, "repo": repo } })
        }
        "baller" | "baller-registry" | "registry" => {
            let url = string_field(&source, "url").ok_or_else(|| {
                BallError::InvalidConfig(
                    "baller registry source requires a 'url' field".to_string(),
                )
            })?;
            json!({ "BallerRegistry": { "url": url } })
        }
        "chocolatey" | "choco" => {
            let feed_url = string_field(&source, "feed_url")
                .or_else(|| string_field(&source, "url"))
                .unwrap_or_else(|| DEFAULT_CHOCOLATEY_FEED.to_string());
            json!({ "Chocolatey": { "feed_url": feed_url } })
        }
        "system" => {
            let manager = string_field(&source, "manager").ok_or_else(|| {
                BallError::InvalidConfig(
                    "system source requires a 'manager' field (apt, dnf, or pacman)".to_string(),
                )
            })?;
            json!({ "System": { "manager": manager } })
        }
        "cargo" | "crate" => {
            let crate_name = string_field(&source, "crate_name")
                .or_else(|| string_field(&source, "name"))
                .or_else(|| map.get("name").and_then(|v| v.as_str().map(String::from)))
                .ok_or_else(|| {
                    BallError::InvalidConfig(
                        "cargo source requires a 'crate_name' field".to_string(),
                    )
                })?;
            json!({ "Cargo": { "crate_name": crate_name } })
        }
        other => {
            return Err(BallError::InvalidConfig(format!(
                "unknown source type '{}': expected github, baller, chocolatey, system, or cargo",
                other
            )))
        }
    };

    map.insert("source".to_string(), normalized);
    Ok(())
}

/// Resolve the owner/repo pair for a github source, falling back to the
/// source `url`, the manifest `repository` URL, then the package name.
fn github_owner_repo(
    source: &Map<String, Value>,
    root: &Map<String, Value>,
) -> Result<(String, String), BallError> {
    let mut owner = string_field(source, "owner");
    let mut repo = string_field(source, "repo");

    if owner.is_none() || repo.is_none() {
        let url = string_field(source, "url").or_else(|| string_field(root, "repository"));
        if let Some(url) = url {
            if let Some((url_owner, url_repo)) = parse_github_url(&url) {
                owner = owner.or(Some(url_owner));
                repo = repo.or(Some(url_repo));
            }
        }
    }

    if repo.is_none() {
        repo = string_field(root, "name");
    }

    match (owner, repo) {
        (Some(owner), Some(repo)) => Ok((owner, repo)),
        _ => Err(BallError::InvalidConfig(
            "github source requires 'owner' and 'repo' (or a github.com repository URL)"
                .to_string(),
        )),
    }
}

pub(crate) fn parse_github_url(url: &str) -> Option<(String, String)> {
    let rest = url.split_once("github.com")?.1;
    let mut parts = rest
        .trim_start_matches([':', '/'])
        .split('/')
        .filter(|part| !part.is_empty());

    let owner = parts.next()?.trim().to_string();
    let repo = parts.next()?.trim().trim_end_matches(".git").to_string();

    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some((owner, repo))
}

fn string_field(map: &Map<String, Value>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(Value::String(value)) if !value.trim().is_empty() => Some(value.trim().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::package::PackageSource;

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

    #[test]
    fn test_parse_nested_toml_full() {
        let toml = r#"
name = "ripgrep"
version = "14.1.0"
description = "Blazingly fast search tool"
author = "Andrew Gallant"
repository = "https://github.com/BurntSushi/ripgrep"

[source]
type = "github"
owner = "BurntSushi"
repo = "ripgrep"

[dependencies]
"fd" = "*"
"libc" = ">=0.2.0"
"?suggested-dep" = "^1.0"

[architectures]
supported = ["x86_64", "aarch64"]

[checksum]
sha256 = "e5f0b2a4c1d3f"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        assert_eq!(pkg.name, "ripgrep");
        assert_eq!(pkg.version, "14.1.0");
        assert_eq!(pkg.sha256.unwrap(), "e5f0b2a4c1d3f");
        assert_eq!(
            pkg.architectures.unwrap(),
            vec!["x86_64".to_string(), "aarch64".to_string()]
        );

        let deps = pkg.dependencies.unwrap();
        assert_eq!(deps.len(), 3);
        assert!(deps.contains(&"fd".to_string()));
        assert!(deps.contains(&"libc >=0.2.0".to_string()));
        assert!(deps.contains(&"?suggested-dep ^1.0".to_string()));

        match pkg.source {
            PackageSource::GitHub { owner, repo } => {
                assert_eq!(owner, "BurntSushi");
                assert_eq!(repo, "ripgrep");
            }
            other => panic!("expected GitHub source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_json_full() {
        let json = r#"{
    "name": "ripgrep",
    "version": "14.1.0",
    "source": { "type": "github", "owner": "BurntSushi", "repo": "ripgrep" },
    "dependencies": ["fd", "libc >=0.2.0", "?suggested-dep ^1.0"],
    "checksum": { "sha256": "e5f0b2a4c1d3f" },
    "architectures": { "supported": ["x86_64"] }
}"#;
        let pkg = ManifestParser::parse_json(json).unwrap();
        assert_eq!(pkg.name, "ripgrep");
        assert_eq!(pkg.sha256.unwrap(), "e5f0b2a4c1d3f");
        assert_eq!(pkg.architectures.unwrap(), vec!["x86_64".to_string()]);
        assert_eq!(pkg.dependencies.unwrap().len(), 3);
        match pkg.source {
            PackageSource::GitHub { owner, repo } => {
                assert_eq!(owner, "BurntSushi");
                assert_eq!(repo, "ripgrep");
            }
            other => panic!("expected GitHub source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_json_dependency_table() {
        let json = r#"{
    "name": "pkg",
    "version": "1.0.0",
    "dependencies": { "libc": ">=0.2.0", "?extra": "*" }
}"#;
        let pkg = ManifestParser::parse_json(json).unwrap();
        let deps = pkg.dependencies.unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&"libc >=0.2.0".to_string()));
        assert!(deps.contains(&"?extra".to_string()));
    }

    #[test]
    fn test_parse_nested_source_chocolatey_defaults_feed() {
        let toml = r#"
name = "7zip"
version = "23.1.0"

[source]
type = "chocolatey"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        match pkg.source {
            PackageSource::Chocolatey { feed_url } => {
                assert_eq!(feed_url, DEFAULT_CHOCOLATEY_FEED);
            }
            other => panic!("expected Chocolatey source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_source_chocolatey_custom_feed() {
        let toml = r#"
name = "7zip"
version = "23.1.0"

[source]
type = "choco"
feed_url = "https://internal.example.com/api/v2"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        match pkg.source {
            PackageSource::Chocolatey { feed_url } => {
                assert_eq!(feed_url, "https://internal.example.com/api/v2");
            }
            other => panic!("expected Chocolatey source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_source_system() {
        let toml = r#"
name = "ripgrep"
version = "13.0.0"

[source]
type = "system"
manager = "apt"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        match pkg.source {
            PackageSource::System { manager } => assert_eq!(manager, "apt"),
            other => panic!("expected System source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_source_system_missing_manager() {
        let toml = r#"
name = "ripgrep"
version = "13.0.0"

[source]
type = "system"
"#;
        let result = ManifestParser::parse_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_nested_source_cargo() {
        let toml = r#"
name = "ripgrep"
version = "14.1.1"

[source]
type = "cargo"
crate_name = "ripgrep"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        match pkg.source {
            PackageSource::Cargo { crate_name } => assert_eq!(crate_name, "ripgrep"),
            other => panic!("expected Cargo source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_source_cargo_defaults_to_package_name() {
        let toml = r#"
name = "ripgrep"
version = "14.1.1"

[source]
type = "crate"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        match pkg.source {
            PackageSource::Cargo { crate_name } => assert_eq!(crate_name, "ripgrep"),
            other => panic!("expected Cargo source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_source_unknown_type_lists_cargo() {
        let toml = r#"
name = "pkg"
version = "1.0.0"

[source]
type = "npm"
"#;
        let err = ManifestParser::parse_toml(toml).unwrap_err();
        let message = format!("{}", err);
        assert!(message.contains("unknown source type 'npm'"));
        assert!(message.contains("cargo"));
    }

    #[test]
    fn test_parse_nested_source_baller() {
        let toml = r#"
name = "pkg"
version = "1.0.0"

[source]
type = "baller"
url = "https://registry.baller.dev/api"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        match pkg.source {
            PackageSource::BallerRegistry { url } => {
                assert_eq!(url, "https://registry.baller.dev/api");
            }
            other => panic!("expected BallerRegistry source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_source_baller_missing_url() {
        let toml = r#"
name = "pkg"
version = "1.0.0"

[source]
type = "baller"
"#;
        let result = ManifestParser::parse_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_nested_source_unknown_type() {
        let toml = r#"
name = "pkg"
version = "1.0.0"

[source]
type = "npm"
"#;
        let result = ManifestParser::parse_toml(toml);
        assert!(result.is_err());
        match result.unwrap_err() {
            BallError::InvalidConfig(msg) => assert!(msg.contains("npm")),
            other => panic!("expected InvalidConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_github_source_from_repository_url() {
        let toml = r#"
name = "ripgrep"
version = "14.1.0"
repository = "https://github.com/BurntSushi/ripgrep"

[source]
type = "github"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        match pkg.source {
            PackageSource::GitHub { owner, repo } => {
                assert_eq!(owner, "BurntSushi");
                assert_eq!(repo, "ripgrep");
            }
            other => panic!("expected GitHub source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_github_source_missing_owner() {
        let toml = r#"
name = "ripgrep"
version = "14.1.0"

[source]
type = "github"
"#;
        let result = ManifestParser::parse_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_nested_architectures_without_supported() {
        let toml = r#"
name = "pkg"
version = "1.0.0"

[architectures]
notes = "unspecified"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        assert!(pkg.architectures.is_none());
    }

    #[test]
    fn test_flat_sha256_wins_over_checksum_table() {
        let toml = r#"
name = "pkg"
version = "1.0.0"
sha256 = "flat-hash"

[checksum]
sha256 = "nested-hash"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        assert_eq!(pkg.sha256.unwrap(), "flat-hash");
    }

    #[test]
    fn test_checksum_algorithm_maps_to_hash_algorithm() {
        let toml = r#"
name = "pkg"
version = "1.0.0"

[checksum]
sha256 = "hash"
algorithm = "SHA512"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        assert_eq!(pkg.hash_algorithm.unwrap(), "SHA512");
    }

    #[test]
    fn test_nested_manifest_round_trips_flat() {
        let toml = r#"
name = "ripgrep"
version = "14.1.0"

[source]
type = "github"
owner = "BurntSushi"
repo = "ripgrep"

[checksum]
sha256 = "hash"
"#;
        let pkg = ManifestParser::parse_toml(toml).unwrap();
        let serialized = ManifestParser::serialize(&pkg, "toml").unwrap();
        assert!(serialized.contains("sha256 = \"hash\""));
        assert!(serialized.contains("[source.GitHub]"));
        assert!(!serialized.contains("type ="));

        let reparsed = ManifestParser::parse_toml(&serialized).unwrap();
        assert_eq!(reparsed.name, "ripgrep");
        assert_eq!(reparsed.source, pkg.source);
    }

    #[test]
    fn test_dependency_table_rejects_non_string_constraint() {
        let json = r#"{
    "name": "pkg",
    "version": "1.0.0",
    "dependencies": { "libc": 42 }
}"#;
        let result = ManifestParser::parse_json(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_github_url_variants() {
        assert_eq!(
            parse_github_url("https://github.com/owner/repo"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_url("git@github.com:owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_url("https://github.com/owner/repo/releases"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(parse_github_url("https://gitlab.com/owner/repo"), None);
        assert_eq!(parse_github_url("https://github.com/owner"), None);
    }
}
