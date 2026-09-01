use std::process::Command;

use crate::core::package::{Package, PackageSource};
use crate::error::error::BallError;

/// Install a crate from crates.io with `cargo install`.
///
/// Runs without `sudo`: cargo builds into the user's `~/.cargo/bin`.
pub fn install_cargo_package(crate_name: &str) -> Result<(), BallError> {
    let status = Command::new("cargo")
        .args(["install", crate_name])
        .status()
        .map_err(|e| BallError::PackageManagerError(format!("failed to run cargo: {}", e)))?;

    if !status.success() {
        return Err(BallError::PackageManagerError(format!(
            "cargo install of '{}' exited with status {}",
            crate_name, status
        )));
    }

    Ok(())
}

pub struct CargoRegistry {
    available: bool,
}

impl CargoRegistry {
    pub fn detect() -> Self {
        Self::from_probe(probe_cargo())
    }

    /// Build a registry from an already-known probe result
    fn from_probe(available: bool) -> Self {
        Self { available }
    }

    /// The CLI name backing this source, if cargo is installed
    pub fn manager_name(&self) -> Option<&'static str> {
        if self.available {
            Some("cargo")
        } else {
            None
        }
    }

    pub fn fetch_package(&self, name: &str) -> Result<Package, BallError> {
        self.ensure_available()?;

        match Self::run_cmd("cargo", &["info", name]) {
            Ok(output) => parse_cargo_info(name, &output),
            Err(_) => {
                let output = Self::run_cmd("cargo", &["search", name, "--limit", "20"])?;
                parse_cargo_search_output(&output)
                    .into_iter()
                    .find(|pkg| pkg.name == name)
                    .ok_or_else(|| BallError::PackageNotFound(name.to_string()))
            }
        }
    }

    pub fn search(&self, query: &str) -> Result<Vec<Package>, BallError> {
        self.ensure_available()?;

        let output = Self::run_cmd("cargo", &["search", query, "--limit", "20"])?;

        Ok(parse_cargo_search_output(&output))
    }

    fn ensure_available(&self) -> Result<(), BallError> {
        if self.available {
            Ok(())
        } else {
            Err(BallError::PackageManagerError(
                "cargo is not installed on this host".to_string(),
            ))
        }
    }

    fn run_cmd(cmd: &str, args: &[&str]) -> Result<String, BallError> {
        let output = Command::new(cmd)
            .args(args)
            .output()
            .map_err(|e| BallError::PackageManagerError(format!("failed to run {}: {}", cmd, e)))?;

        if !output.status.success() {
            return Err(BallError::PackageManagerError(format!(
                "{} exited with status {}",
                cmd, output.status
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

fn probe_cargo() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Fields `cargo info` prints as `key: value` after the description block
const INFO_FIELDS: [&str; 6] = [
    "version",
    "license",
    "rust-version",
    "documentation",
    "homepage",
    "repository",
];

fn info_field(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(": ")?;
    if INFO_FIELDS.contains(&key) {
        Some((key, value.trim()))
    } else {
        None
    }
}

/// Take the version `cargo install` would land on.
///
/// `cargo info` prints `1.0.228 (latest 1.0.229)` when a lockfile pins an older
/// release than crates.io has; the latest is what an install resolves to.
fn parse_info_version(value: &str) -> String {
    if let Some(rest) = value.split_once("(latest ") {
        return rest.1.trim_end_matches(')').trim().to_string();
    }

    value
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

fn parse_cargo_info(name: &str, output: &str) -> Result<Package, BallError> {
    if output.trim().is_empty() {
        return Err(BallError::PackageNotFound(name.to_string()));
    }

    let mut crate_name = String::new();
    let mut version = String::new();
    let mut description = String::new();
    let mut repository = String::new();
    let mut in_header = true;

    for line in output.lines() {
        if line.starts_with("features:") || line.starts_with("note:") {
            break;
        }

        if let Some((key, value)) = info_field(line) {
            in_header = false;
            match key {
                "version" => version = parse_info_version(value),
                "repository" => repository = value.to_string(),
                _ => {}
            }
            continue;
        }

        if !in_header || line.trim().is_empty() {
            continue;
        }

        if crate_name.is_empty() {
            crate_name = line
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string();
        } else {
            let text = line.trim();
            if !description.is_empty() {
                description.push(' ');
            }
            description.push_str(text);
        }
    }

    if version.is_empty() {
        return Err(BallError::PackageNotFound(name.to_string()));
    }

    if crate_name.is_empty() {
        crate_name = name.to_string();
    }

    Ok(Package {
        name: crate_name.clone(),
        version,
        description: if description.is_empty() {
            None
        } else {
            Some(description)
        },
        author: None,
        repository: if repository.is_empty() {
            None
        } else {
            Some(repository)
        },
        architectures: None,
        dependencies: None,
        sha256: None,
        hash_algorithm: None,
        download_url: None,
        source: PackageSource::Cargo { crate_name },
    })
}

/// Parse `cargo search` lines of the form `name = "version"    # description`
fn parse_cargo_search_output(output: &str) -> Vec<Package> {
    let mut results = Vec::new();

    for line in output.lines() {
        let Some((name, rest)) = line.split_once(" = ") else {
            continue;
        };

        let name = name.trim();
        if name.is_empty() {
            continue;
        }

        let Some(version) = rest
            .trim()
            .strip_prefix('"')
            .and_then(|rest| rest.split_once('"'))
            .map(|(version, _)| version)
        else {
            continue;
        };

        let description = rest
            .split_once('#')
            .map(|(_, desc)| desc.trim().to_string())
            .filter(|desc| !desc.is_empty());

        results.push(Package {
            name: name.to_string(),
            version: version.to_string(),
            description,
            author: None,
            repository: None,
            architectures: None,
            dependencies: None,
            sha256: None,
            hash_algorithm: None,
            download_url: None,
            source: PackageSource::Cargo {
                crate_name: name.to_string(),
            },
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_OUTPUT: &str = r#"serde = "1.0.229"         # A generic serialization/deserialization framework
ripgrep = "14.1.1"        # Line oriented search tool
bat = "0.24.0"
... and 20977 crates more (use --limit N to see more)
note: to learn more about a package, run `cargo info <name>`
"#;

    const INFO_OUTPUT: &str = r#"serde #serde #serialization #no_std
A generic serialization/deserialization framework
version: 1.0.228 (latest 1.0.229)
license: MIT OR Apache-2.0
rust-version: 1.56
documentation: https://docs.rs/serde
homepage: https://serde.rs
repository: https://github.com/serde-rs/serde
crates.io: https://crates.io/crates/serde/1.0.228
features:
 +default      = [std]
  std          = [serde_core/std]
note: to see how you depend on serde, run `cargo tree --invert serde@1.0.228`
"#;

    #[test]
    fn test_parse_cargo_search_output() {
        let results = parse_cargo_search_output(SEARCH_OUTPUT);

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].name, "serde");
        assert_eq!(results[0].version, "1.0.229");
        assert_eq!(
            results[0].description.as_deref(),
            Some("A generic serialization/deserialization framework")
        );
        assert_eq!(results[1].name, "ripgrep");
        assert_eq!(results[1].version, "14.1.1");
        assert_eq!(results[2].name, "bat");
        assert_eq!(results[2].description, None);
    }

    #[test]
    fn test_parse_cargo_search_output_stamps_cargo_source() {
        let results = parse_cargo_search_output(SEARCH_OUTPUT);

        match &results[0].source {
            PackageSource::Cargo { crate_name } => assert_eq!(crate_name, "serde"),
            other => panic!("expected Cargo source, got {:?}", other),
        }
        assert!(results.iter().all(|pkg| pkg.download_url.is_none()));
        assert!(results.iter().all(|pkg| pkg.sha256.is_none()));
    }

    #[test]
    fn test_parse_cargo_search_output_skips_notes_and_totals() {
        let results = parse_cargo_search_output(SEARCH_OUTPUT);

        assert!(!results.iter().any(|pkg| pkg.name.starts_with("...")));
        assert!(!results.iter().any(|pkg| pkg.name.starts_with("note")));
    }

    #[test]
    fn test_parse_cargo_info() {
        let pkg = parse_cargo_info("serde", INFO_OUTPUT).unwrap();

        assert_eq!(pkg.name, "serde");
        assert_eq!(pkg.version, "1.0.229");
        assert_eq!(
            pkg.description.as_deref(),
            Some("A generic serialization/deserialization framework")
        );
        assert_eq!(
            pkg.repository.as_deref(),
            Some("https://github.com/serde-rs/serde")
        );
        assert_eq!(pkg.download_url, None);
        assert_eq!(pkg.sha256, None);
        match &pkg.source {
            PackageSource::Cargo { crate_name } => assert_eq!(crate_name, "serde"),
            other => panic!("expected Cargo source, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_info_version_without_latest() {
        assert_eq!(parse_info_version("1.0.229"), "1.0.229");
        assert_eq!(parse_info_version("1.0.228 (latest 1.0.229)"), "1.0.229");
    }

    #[test]
    fn test_parse_cargo_info_keeps_description_containing_colon() {
        let output = "ripgrep\nrecursively searches: fast and friendly\nversion: 14.1.1\n";
        let pkg = parse_cargo_info("ripgrep", output).unwrap();

        assert_eq!(
            pkg.description.as_deref(),
            Some("recursively searches: fast and friendly")
        );
        assert_eq!(pkg.version, "14.1.1");
        assert_eq!(pkg.repository, None);
    }

    #[test]
    fn test_parse_cargo_info_empty_output_is_not_found() {
        let err = parse_cargo_info("nope", "   \n").unwrap_err();
        assert!(matches!(err, BallError::PackageNotFound(_)));
    }

    #[test]
    fn test_parse_cargo_info_without_version_is_not_found() {
        let err = parse_cargo_info("nope", "nope\nsome description\n").unwrap_err();
        assert!(matches!(err, BallError::PackageNotFound(_)));
    }

    #[test]
    fn test_available_registry_reports_manager() {
        let registry = CargoRegistry::from_probe(true);
        assert_eq!(registry.manager_name(), Some("cargo"));
    }

    #[test]
    fn test_unavailable_registry_has_no_manager() {
        let registry = CargoRegistry::from_probe(false);
        assert_eq!(registry.manager_name(), None);
    }

    #[test]
    fn test_unavailable_registry_errors_on_fetch_and_search() {
        let registry = CargoRegistry::from_probe(false);

        assert!(matches!(
            registry.fetch_package("serde").unwrap_err(),
            BallError::PackageManagerError(_)
        ));
        assert!(matches!(
            registry.search("serde").unwrap_err(),
            BallError::PackageManagerError(_)
        ));
    }
}
