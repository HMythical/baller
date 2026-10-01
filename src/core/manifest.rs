use crate::core::package::{Package, Platform};
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
        Self::parse_toml_on(content, Platform::host())
    }

    /// [`ManifestParser::parse_toml`] as `host` sees it: `host`'s
    /// `[source.<os>]` table, if any, is the one selected.
    fn parse_toml_on(content: &str, host: Platform) -> Result<Package, BallError> {
        let raw: toml::Value = toml::from_str(content).map_err(|e| {
            BallError::InvalidConfig(format!("Failed to parse TOML manifest: {}", e))
        })?;
        let value = serde_json::to_value(raw).map_err(|e| {
            BallError::InvalidConfig(format!("Failed to parse TOML manifest: {}", e))
        })?;
        Self::from_value(value, "TOML", host)
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
        Self::parse_json_on(content, Platform::host())
    }

    /// [`ManifestParser::parse_json`] as `host` sees it
    fn parse_json_on(content: &str, host: Platform) -> Result<Package, BallError> {
        let value: Value = serde_json::from_str(content).map_err(|e| {
            BallError::InvalidConfig(format!("Failed to parse JSON manifest: {}", e))
        })?;
        Self::from_value(value, "JSON", host)
    }

    /// Normalize a decoded manifest into the flat `Package` shape
    fn from_value(value: Value, format: &str, host: Platform) -> Result<Package, BallError> {
        let normalized = normalize_manifest(value, host)?;
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
/// `[source.<os>]` tables are resolved here, for `host`, so `Package` stays
/// flat and no consumer needs platform-selection logic of its own.
fn normalize_manifest(value: Value, host: Platform) -> Result<Value, BallError> {
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
    normalize_source(&mut map, host)?;

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

/// The `PackageSource` variant names, as the flat layout spells them:
/// `[source] GitHub = { .. }`, or `[source.GitHub]` as serialization emits it.
const SOURCE_TAGS: [&str; 5] = ["GitHub", "BallerRegistry", "Chocolatey", "System", "Cargo"];

/// Package fields a `[source.<os>]` table may override, typed or not
const PLATFORM_ARTIFACT_FIELDS: [&str; 3] = ["download_url", "sha256", "hash_algorithm"];

/// One validated `[source.<os>]` table
struct PlatformEntry {
    /// The normalized source, when the table declares one (`type` or a flat tag)
    source: Option<Value>,
    /// `download_url` / `sha256` / `hash_algorithm` overrides for the package
    overrides: Map<String, Value>,
}

/// `[source] type = ".."` becomes the tagged `PackageSource` representation,
/// with `host`'s `[source.<os>]` table applied first.
///
/// A plain `[source]` is the fallback on every platform. A `[source.linux]` or
/// `[source.windows]` table that declares a source (`type`, or a flat tag)
/// replaces it wholesale on that platform; one that declares none inherits it
/// and may only override `download_url`, `sha256` and `hash_algorithm`. Every
/// platform table is validated whichever host parses it, so a typo fails on
/// both platforms rather than only on the one it targets.
fn normalize_source(map: &mut Map<String, Value>, host: Platform) -> Result<(), BallError> {
    let table = match map.get("source") {
        Some(Value::Object(table)) => table.clone(),
        Some(_) => {
            return Err(BallError::InvalidConfig(
                "manifest 'source' must be a table".to_string(),
            ))
        }
        None => return Ok(()),
    };

    let mut fallback = Map::new();
    let mut selected = None;
    let mut has_platform_tables = false;

    for (key, value) in table {
        if let Some(platform) = Platform::from_name(&key) {
            let entry = match &value {
                Value::Object(entry) => parse_platform_entry(&key, entry, map)?,
                _ => {
                    return Err(BallError::InvalidConfig(format!(
                        "[source.{}] must be a table",
                        key
                    )))
                }
            };
            has_platform_tables = true;
            if platform == host {
                selected = Some(entry);
            }
        } else if value.is_object() && !SOURCE_TAGS.contains(&key.as_str()) {
            return Err(BallError::InvalidConfig(format!(
                "unknown platform table [source.{}]: expected [source.linux] or [source.windows] \
                 (flat sources are spelled {})",
                key,
                SOURCE_TAGS.join(", ")
            )));
        } else {
            fallback.insert(key, value);
        }
    }

    // No platform tables: exactly the single-source path manifests always took
    if !has_platform_tables {
        let normalized = normalize_source_table(&fallback, map)?;
        map.insert("source".to_string(), normalized);
        return Ok(());
    }

    let fallback = if fallback.is_empty() {
        None
    } else {
        Some(normalize_source_table(&fallback, map)?)
    };

    let (source, overrides) = match selected {
        Some(entry) => (entry.source.or(fallback), entry.overrides),
        None => (fallback, Map::new()),
    };

    for (field, value) in overrides {
        map.insert(field, value);
    }

    // Only other platforms' tables and no fallback: this host gets the default
    // source, exactly as if `[source]` had been omitted.
    match source {
        Some(source) => map.insert("source".to_string(), source),
        None => map.remove("source"),
    };

    Ok(())
}

/// Validate one `[source.<os>]` table and split it into the source it
/// declares (if any) and the artifact fields it overrides.
fn parse_platform_entry(
    key: &str,
    entry: &Map<String, Value>,
    root: &Map<String, Value>,
) -> Result<PlatformEntry, BallError> {
    if entry.is_empty() {
        return Err(BallError::InvalidConfig(format!(
            "[source.{}] is empty: declare a source 'type', or override {}",
            key,
            PLATFORM_ARTIFACT_FIELDS.join(", ")
        )));
    }

    let mut overrides = Map::new();
    let mut source_fields = Map::new();
    for (field, value) in entry {
        if !PLATFORM_ARTIFACT_FIELDS.contains(&field.as_str()) {
            source_fields.insert(field.clone(), value.clone());
            continue;
        }
        match value {
            Value::String(text) if !text.trim().is_empty() => {
                overrides.insert(field.clone(), value.clone());
            }
            _ => {
                return Err(BallError::InvalidConfig(format!(
                    "[source.{}] '{}' must be a non-empty string",
                    key, field
                )))
            }
        }
    }

    let declares_source = source_fields.contains_key("type")
        || source_fields
            .keys()
            .any(|field| SOURCE_TAGS.contains(&field.as_str()));
    if declares_source {
        return Ok(PlatformEntry {
            source: Some(normalize_source_table(&source_fields, root)?),
            overrides,
        });
    }

    // Source fields with no type would yield a half-populated source
    if let Some(field) = source_fields.keys().next() {
        return Err(BallError::InvalidConfig(format!(
            "[source.{}] sets '{}' without a source 'type': add one to replace [source] on {}, \
             or move '{}' under [source]",
            key, field, key, field
        )));
    }

    Ok(PlatformEntry {
        source: None,
        overrides,
    })
}

/// Normalize a single source table through the one `type` switch that knows
/// how each source is spelled. A table with no `type` is the flat tagged form
/// and passes through for serde to decode.
fn normalize_source_table(
    source: &Map<String, Value>,
    map: &Map<String, Value>,
) -> Result<Value, BallError> {
    let source_type = match source.get("type") {
        Some(Value::String(source_type)) => source_type.trim().to_lowercase(),
        Some(_) => {
            return Err(BallError::InvalidConfig(
                "source 'type' must be a string".to_string(),
            ))
        }
        None => return Ok(Value::Object(source.clone())),
    };

    let normalized = match source_type.as_str() {
        "github" => {
            let (owner, repo) = github_owner_repo(source, map)?;
            json!({ "GitHub": { "owner": owner, "repo": repo } })
        }
        "baller" | "baller-registry" | "registry" => {
            let url = string_field(source, "url").ok_or_else(|| {
                BallError::InvalidConfig(
                    "baller registry source requires a 'url' field".to_string(),
                )
            })?;
            json!({ "BallerRegistry": { "url": url } })
        }
        "chocolatey" | "choco" => {
            let feed_url = string_field(source, "feed_url")
                .or_else(|| string_field(source, "url"))
                .unwrap_or_else(|| DEFAULT_CHOCOLATEY_FEED.to_string());
            json!({ "Chocolatey": { "feed_url": feed_url } })
        }
        "system" => {
            let manager = string_field(source, "manager").ok_or_else(|| {
                BallError::InvalidConfig(
                    "system source requires a 'manager' field (apt, dnf, or pacman)".to_string(),
                )
            })?;
            json!({ "System": { "manager": manager } })
        }
        "cargo" | "crate" => {
            let crate_name = string_field(source, "crate_name")
                .or_else(|| string_field(source, "name"))
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

    Ok(normalized)
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

    /// `Package` holds one source, so serialization emits the source already
    /// selected for the parsing host and drops the other platform's table —
    /// by design, and documented in docs/manifest.md.
    #[test]
    fn test_platform_manifest_round_trips_as_the_selected_flat_source() {
        let toml = r#"
name = "tool"
version = "1.0.0"

[source]
type = "github"
owner = "o"
repo = "tool"

[source.linux]
type = "system"
manager = "apt"

[source.windows]
type = "chocolatey"
download_url = "https://example.com/tool.nupkg"
sha256 = "win-hash"
"#;
        for host in [Platform::Linux, Platform::Windows] {
            let pkg = ManifestParser::parse_toml_on(toml, host).unwrap();
            let serialized = ManifestParser::serialize(&pkg, "toml").unwrap();
            assert!(!serialized.contains("linux"), "{}", serialized);
            assert!(!serialized.contains("windows"), "{}", serialized);
            assert!(!serialized.contains("type ="), "{}", serialized);

            // The emitted flat tag must not be mistaken for a platform table
            for reparse_host in [Platform::Linux, Platform::Windows] {
                let reparsed = ManifestParser::parse_toml_on(&serialized, reparse_host).unwrap();
                assert_eq!(reparsed.source, pkg.source);
                assert_eq!(reparsed.download_url, pkg.download_url);
                assert_eq!(reparsed.sha256, pkg.sha256);
            }

            let json = ManifestParser::serialize(&pkg, "json").unwrap();
            let reparsed = ManifestParser::parse_json_on(&json, host).unwrap();
            assert_eq!(reparsed.source, pkg.source);
        }
    }

    const PLATFORM_TOML: &str = r#"
name = "tool"
version = "1.0.0"
download_url = "https://example.com/tool-generic.tar.gz"
sha256 = "generic-hash"

[source]
type = "github"
owner = "o"
repo = "tool"

[source.linux]
type = "system"
manager = "apt"

[source.windows]
type = "chocolatey"
download_url = "https://example.com/tool.nupkg"
sha256 = "win-hash"
"#;

    fn github() -> PackageSource {
        PackageSource::GitHub {
            owner: "o".to_string(),
            repo: "tool".to_string(),
        }
    }

    fn apt() -> PackageSource {
        PackageSource::System {
            manager: "apt".to_string(),
        }
    }

    fn default_chocolatey() -> PackageSource {
        PackageSource::Chocolatey {
            feed_url: DEFAULT_CHOCOLATEY_FEED.to_string(),
        }
    }

    fn invalid_config(result: Result<Package, BallError>) -> String {
        match result {
            Err(BallError::InvalidConfig(msg)) => msg,
            other => panic!("expected InvalidConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_platform_tables_select_the_host_entry() {
        let linux = ManifestParser::parse_toml_on(PLATFORM_TOML, Platform::Linux).unwrap();
        assert_eq!(linux.source, apt());
        assert_eq!(
            linux.download_url.as_deref(),
            Some("https://example.com/tool-generic.tar.gz")
        );
        assert_eq!(linux.sha256.as_deref(), Some("generic-hash"));

        let windows = ManifestParser::parse_toml_on(PLATFORM_TOML, Platform::Windows).unwrap();
        assert_eq!(windows.source, default_chocolatey());
        assert_eq!(
            windows.download_url.as_deref(),
            Some("https://example.com/tool.nupkg")
        );
        assert_eq!(windows.sha256.as_deref(), Some("win-hash"));
    }

    #[test]
    fn test_platform_tables_in_json_use_the_same_shape() {
        let json = r#"{
    "name": "tool",
    "version": "1.0.0",
    "source": {
        "type": "github", "owner": "o", "repo": "tool",
        "linux": { "type": "system", "manager": "apt" },
        "windows": { "type": "chocolatey", "sha256": "win-hash" }
    }
}"#;
        let linux = ManifestParser::parse_json_on(json, Platform::Linux).unwrap();
        assert_eq!(linux.source, apt());
        assert_eq!(linux.sha256, None);

        let windows = ManifestParser::parse_json_on(json, Platform::Windows).unwrap();
        assert_eq!(windows.source, default_chocolatey());
        assert_eq!(windows.sha256.as_deref(), Some("win-hash"));

        let bad = r#"{"name": "tool", "version": "1.0.0", "source": {"mac": {"type": "github"}}}"#;
        let msg = invalid_config(ManifestParser::parse_json_on(bad, Platform::Linux));
        assert!(msg.contains("[source.mac]"), "{}", msg);
    }

    #[test]
    fn test_linux_table_alone() {
        let toml = r#"
name = "tool"
version = "1.0.0"

[source.linux]
type = "system"
manager = "apt"
"#;
        let linux = ManifestParser::parse_toml_on(toml, Platform::Linux).unwrap();
        assert_eq!(linux.source, apt());

        // No fallback and no Windows table: the default source, as if
        // `[source]` had been omitted entirely
        let windows = ManifestParser::parse_toml_on(toml, Platform::Windows).unwrap();
        assert_eq!(windows.source, PackageSource::default());
    }

    #[test]
    fn test_windows_table_alone() {
        let toml = r#"
name = "tool"
version = "1.0.0"

[source.windows]
type = "choco"
"#;
        let windows = ManifestParser::parse_toml_on(toml, Platform::Windows).unwrap();
        assert_eq!(windows.source, default_chocolatey());

        let linux = ManifestParser::parse_toml_on(toml, Platform::Linux).unwrap();
        assert_eq!(linux.source, PackageSource::default());
    }

    #[test]
    fn test_fallback_is_used_where_no_platform_table_applies() {
        let toml = r#"
name = "tool"
version = "1.0.0"

[source]
type = "github"
owner = "o"
repo = "tool"

[source.windows]
type = "chocolatey"
"#;
        let linux = ManifestParser::parse_toml_on(toml, Platform::Linux).unwrap();
        assert_eq!(linux.source, github());
    }

    #[test]
    fn test_no_platform_tables_parse_identically_on_both_hosts() {
        let toml = r#"
name = "tool"
version = "1.0.0"

[source]
type = "chocolatey"
"#;
        // Parsing stays host-agnostic for legacy manifests; the platform gate,
        // not the parser, is what rejects this on Linux.
        for host in [Platform::Linux, Platform::Windows] {
            let pkg = ManifestParser::parse_toml_on(toml, host).unwrap();
            assert_eq!(pkg.source, default_chocolatey());
        }
    }

    #[test]
    fn test_typed_platform_table_replaces_the_fallback_wholesale() {
        // The fallback's github `url` must not leak into chocolatey's
        // `feed_url`, which also accepts a `url` spelling
        let toml = r#"
name = "tool"
version = "1.0.0"

[source]
type = "github"
url = "https://github.com/o/tool"

[source.windows]
type = "chocolatey"
"#;
        let windows = ManifestParser::parse_toml_on(toml, Platform::Windows).unwrap();
        assert_eq!(windows.source, default_chocolatey());
    }

    #[test]
    fn test_untyped_platform_table_inherits_and_overrides_download_url() {
        let toml = r#"
name = "tool"
version = "1.0.0"
download_url = "https://example.com/tool-generic.tar.gz"

[source]
type = "github"
owner = "o"
repo = "tool"

[source.linux]
download_url = "https://example.com/tool-linux.tar.gz"
"#;
        let linux = ManifestParser::parse_toml_on(toml, Platform::Linux).unwrap();
        assert_eq!(linux.source, github());
        assert_eq!(
            linux.download_url.as_deref(),
            Some("https://example.com/tool-linux.tar.gz")
        );

        let windows = ManifestParser::parse_toml_on(toml, Platform::Windows).unwrap();
        assert_eq!(windows.source, github());
        assert_eq!(
            windows.download_url.as_deref(),
            Some("https://example.com/tool-generic.tar.gz")
        );
    }

    #[test]
    fn test_platform_sha256_overrides_flat_and_checksum_table() {
        let toml = r#"
name = "tool"
version = "1.0.0"

[checksum]
sha256 = "table-hash"
algorithm = "SHA512"

[source]
type = "github"
owner = "o"
repo = "tool"

[source.linux]
sha256 = "linux-hash"

[source.windows]
sha256 = "windows-hash"
hash_algorithm = "SHA256"
"#;
        let linux = ManifestParser::parse_toml_on(toml, Platform::Linux).unwrap();
        assert_eq!(linux.sha256.as_deref(), Some("linux-hash"));
        // Fields the platform table does not set are inherited
        assert_eq!(linux.hash_algorithm.as_deref(), Some("SHA512"));

        let windows = ManifestParser::parse_toml_on(toml, Platform::Windows).unwrap();
        assert_eq!(windows.sha256.as_deref(), Some("windows-hash"));
        assert_eq!(windows.hash_algorithm.as_deref(), Some("SHA256"));
    }

    #[test]
    fn test_platform_table_may_use_the_flat_tagged_form() {
        let toml = r#"
name = "tool"
version = "1.0.0"

[source.linux.System]
manager = "apt"
"#;
        let linux = ManifestParser::parse_toml_on(toml, Platform::Linux).unwrap();
        assert_eq!(linux.source, apt());
    }

    #[test]
    fn test_unknown_platform_key_is_rejected_by_name() {
        for key in ["mac", "win", "Linux", "github"] {
            let toml = format!(
                "name = \"tool\"\nversion = \"1.0.0\"\n\n[source.{}]\ntype = \"github\"\n",
                key
            );
            for host in [Platform::Linux, Platform::Windows] {
                let msg = invalid_config(ManifestParser::parse_toml_on(&toml, host));
                assert!(msg.contains(&format!("[source.{}]", key)), "{}", msg);
                assert!(
                    msg.contains("[source.linux] or [source.windows]"),
                    "{}",
                    msg
                );
            }
        }
    }

    #[test]
    fn test_empty_platform_table_is_rejected() {
        let toml = "name = \"tool\"\nversion = \"1.0.0\"\n\n[source.linux]\n";
        // Rejected on the other host too, not only where it would apply
        for host in [Platform::Linux, Platform::Windows] {
            let msg = invalid_config(ManifestParser::parse_toml_on(toml, host));
            assert!(msg.contains("[source.linux] is empty"), "{}", msg);
        }
    }

    #[test]
    fn test_untyped_platform_table_with_source_fields_is_rejected() {
        let toml = r#"
name = "tool"
version = "1.0.0"

[source]
type = "github"
owner = "o"
repo = "tool"

[source.linux]
manager = "apt"
"#;
        let msg = invalid_config(ManifestParser::parse_toml_on(toml, Platform::Windows));
        assert!(msg.contains("[source.linux] sets 'manager'"), "{}", msg);
        assert!(msg.contains("without a source 'type'"), "{}", msg);
    }

    #[test]
    fn test_platform_table_errors_surface_on_every_host() {
        let bad_type = "name = \"t\"\nversion = \"1\"\n\n[source.windows]\ntype = \"npm\"\n";
        let msg = invalid_config(ManifestParser::parse_toml_on(bad_type, Platform::Linux));
        assert!(msg.contains("unknown source type 'npm'"), "{}", msg);

        let no_manager = "name = \"t\"\nversion = \"1\"\n\n[source.linux]\ntype = \"system\"\n";
        let msg = invalid_config(ManifestParser::parse_toml_on(no_manager, Platform::Windows));
        assert!(msg.contains("requires a 'manager' field"), "{}", msg);

        let blank_url = "name = \"t\"\nversion = \"1\"\n\n[source.linux]\ndownload_url = \" \"\n";
        let msg = invalid_config(ManifestParser::parse_toml_on(blank_url, Platform::Windows));
        assert!(
            msg.contains("'download_url' must be a non-empty string"),
            "{}",
            msg
        );
    }

    #[test]
    fn test_platform_key_must_be_a_table() {
        let json = r#"{"name": "t", "version": "1", "source": {"type": "github", "repo": "t", "owner": "o", "linux": "apt"}}"#;
        let msg = invalid_config(ManifestParser::parse_json_on(json, Platform::Linux));
        assert!(msg.contains("[source.linux] must be a table"), "{}", msg);
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
