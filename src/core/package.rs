use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum PackageSource {
    GitHub { owner: String, repo: String },
    BallerRegistry { url: String },
    Chocolatey { feed_url: String },
    System { manager: String },
    Cargo { crate_name: String },
}

/// An operating system B.A.L.L.E.R. installs onto.
///
/// Only Linux and Windows exist here: any other host is rejected at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
}

impl Platform {
    /// The platform this build of baller runs on
    pub fn host() -> Self {
        if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }

    /// The lowercase name used in messages and as a manifest `[source.<os>]` key
    pub fn name(&self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::Windows => "windows",
        }
    }

    /// Parse a `[source.<os>]` key; only the exact lowercase names match
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "linux" => Some(Platform::Linux),
            "windows" => Some(Platform::Windows),
            _ => None,
        }
    }
}

/// Which platforms a source can serve an installable artifact for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAffinity {
    /// Serves any host: the OS/arch is resolved per artifact, if at all
    Agnostic,
    /// Serves only this platform
    Only(Platform),
}

impl SourceAffinity {
    /// Whether a source with this affinity can install onto `host`
    pub fn supports(&self, host: Platform) -> bool {
        match self {
            SourceAffinity::Agnostic => true,
            SourceAffinity::Only(platform) => *platform == host,
        }
    }

    /// `any`, or the single platform's name
    pub fn name(&self) -> &'static str {
        match self {
            SourceAffinity::Agnostic => "any",
            SourceAffinity::Only(platform) => platform.name(),
        }
    }
}

impl PackageSource {
    /// The platforms this source can serve.
    ///
    /// Deliberately an exhaustive match rather than derived from a source's
    /// name, so a new variant cannot be added without deciding its scope.
    pub fn affinity(&self) -> SourceAffinity {
        match self {
            // Chocolatey/NuGet payloads target Windows
            PackageSource::Chocolatey { .. } => SourceAffinity::Only(Platform::Windows),
            // Driven by /etc/os-release and a distro package manager
            PackageSource::System { .. } => SourceAffinity::Only(Platform::Linux),
            // GitHub picks a host asset per release, the Baller registry decides
            // server-side, and `cargo install` builds for whatever host runs it
            PackageSource::GitHub { .. }
            | PackageSource::BallerRegistry { .. }
            | PackageSource::Cargo { .. } => SourceAffinity::Agnostic,
        }
    }
}

impl Default for PackageSource {
    fn default() -> Self {
        PackageSource::GitHub {
            owner: String::new(),
            repo: String::new(),
        }
    }
}

/// A package stating where its own known-issue surface lives.
///
/// Distribution shape and advisory ecosystem are not the same thing: a tool
/// shipped as a GitHub release may also be published as a crate, and only its
/// author knows that. A manifest `[advisory]` section — and the same field on
/// registry-served metadata — lets them say so, and Referee treats the answer
/// as authoritative rather than guessing from the download URL.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct AdvisoryDeclaration {
    /// OSV ecosystem, e.g. `crates.io`, `npm`, `GitHub`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ecosystem: Option<String>,
    /// The name inside that ecosystem; defaults to the package name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Advisory ids this package is also known by (CVE, GHSA, …)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub repository: Option<String>,

    pub architectures: Option<Vec<String>>,

    pub dependencies: Option<Vec<String>>,

    pub sha256: Option<String>,

    pub hash_algorithm: Option<String>,

    pub download_url: Option<String>,

    #[serde(default)]
    pub source: PackageSource,

    /// The package's self-declared advisory identity, when it ships one.
    ///
    /// Serialised last because TOML requires tables after scalar fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisory: Option<AdvisoryDeclaration>,

    /// Advisories the serving registry published for this exact version.
    ///
    /// OSV-shaped records, kept as raw JSON so the wire format stays the
    /// registry's business and `Package` keeps round-tripping through TOML.
    /// When present, Referee consumes these instead of querying for this
    /// package: the registry is the authority on its own contents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vulnerabilities: Vec<serde_json::Value>,
}

impl Package {
    #[allow(dead_code)]
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            description: None,
            author: None,
            repository: None,
            architectures: None,
            dependencies: None,
            sha256: None,
            hash_algorithm: None,
            download_url: None,
            source: PackageSource::GitHub {
                owner: String::new(),
                repo: name.to_string(),
            },
            advisory: None,
            vulnerabilities: Vec::new(),
        }
    }

    /// The `(ecosystem, name)` pairs this package declares about itself.
    ///
    /// A declaration with no ecosystem names nothing queryable and is dropped:
    /// an ecosystem is what makes an advisory lookup possible at all.
    pub fn advisory_identities(&self) -> Vec<(String, String)> {
        let declaration = match self.advisory.as_ref() {
            Some(declaration) => declaration,
            None => return Vec::new(),
        };

        let ecosystem = match declaration.ecosystem.as_deref().map(str::trim) {
            Some(ecosystem) if !ecosystem.is_empty() => ecosystem.to_string(),
            _ => return Vec::new(),
        };

        let name = declaration
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.name.trim())
            .to_string();

        if name.is_empty() {
            return Vec::new();
        }

        vec![(ecosystem, name)]
    }

    /// Advisory ids the package declares itself to be tracked under.
    pub fn declared_aliases(&self) -> &[String] {
        self.advisory
            .as_ref()
            .map(|declaration| declaration.aliases.as_slice())
            .unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_new() {
        let pkg = Package::new("test-pkg", "1.0.0");
        assert_eq!(pkg.name, "test-pkg");
        assert_eq!(pkg.version, "1.0.0");
        assert_eq!(pkg.description, None);
        assert_eq!(pkg.author, None);
        assert_eq!(pkg.sha256, None);
        assert_eq!(pkg.download_url, None);
        assert_eq!(pkg.dependencies, None);
        match &pkg.source {
            PackageSource::GitHub { owner, repo } => {
                assert_eq!(owner, "");
                assert_eq!(repo, "test-pkg");
            }
            _ => panic!("expected GitHub source"),
        }
    }

    #[test]
    fn test_package_source_default() {
        let source = PackageSource::default();
        match source {
            PackageSource::GitHub { owner, repo } => {
                assert_eq!(owner, "");
                assert_eq!(repo, "");
            }
            _ => panic!("expected GitHub source"),
        }
    }

    #[test]
    fn test_package_serialize_deserialize_toml() {
        let pkg = Package::new("toml-pkg", "2.0.0");
        let toml_str = toml::to_string(&pkg).unwrap();
        let deserialized: Package = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized.name, "toml-pkg");
        assert_eq!(deserialized.version, "2.0.0");
    }

    #[test]
    fn test_package_serialize_deserialize_json() {
        let pkg = Package::new("json-pkg", "3.0.0");
        let json_str = serde_json::to_string(&pkg).unwrap();
        let deserialized: Package = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.name, "json-pkg");
        assert_eq!(deserialized.version, "3.0.0");
    }

    #[test]
    fn test_package_with_all_fields() {
        let pkg = Package {
            name: "full-pkg".to_string(),
            version: "4.0.0".to_string(),
            description: Some("A test package".to_string()),
            author: Some("Test Author".to_string()),
            repository: Some("https://github.com/test/full-pkg".to_string()),
            architectures: Some(vec!["x86_64".to_string()]),
            dependencies: Some(vec!["dep1".to_string(), "dep2 >=1.0".to_string()]),
            sha256: Some("abc123".to_string()),
            hash_algorithm: None,
            download_url: Some("https://example.com/pkg.tar.gz".to_string()),
            source: PackageSource::BallerRegistry {
                url: "https://reg.example.com".to_string(),
            },
            advisory: None,
            vulnerabilities: Vec::new(),
        };

        let json = serde_json::to_string(&pkg).unwrap();
        let deserialized: Package = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "full-pkg");
        assert_eq!(deserialized.version, "4.0.0");
        assert_eq!(deserialized.description.unwrap(), "A test package");
        assert_eq!(deserialized.author.unwrap(), "Test Author");
        assert!(deserialized
            .dependencies
            .unwrap()
            .contains(&"dep1".to_string()));
    }

    #[test]
    fn test_package_source_equality() {
        let a = PackageSource::GitHub {
            owner: "a".to_string(),
            repo: "b".to_string(),
        };
        let b = PackageSource::GitHub {
            owner: "a".to_string(),
            repo: "b".to_string(),
        };
        assert_eq!(a, b);

        let c = PackageSource::BallerRegistry {
            url: "url".to_string(),
        };
        assert_ne!(a, c);
    }

    fn every_source() -> Vec<PackageSource> {
        vec![
            PackageSource::GitHub {
                owner: "o".to_string(),
                repo: "r".to_string(),
            },
            PackageSource::BallerRegistry {
                url: "https://reg.example.com".to_string(),
            },
            PackageSource::Chocolatey {
                feed_url: "https://feed.example.com".to_string(),
            },
            PackageSource::System {
                manager: "apt".to_string(),
            },
            PackageSource::Cargo {
                crate_name: "c".to_string(),
            },
        ]
    }

    #[test]
    fn test_source_affinity_mapping() {
        let affinities: Vec<SourceAffinity> =
            every_source().iter().map(PackageSource::affinity).collect();
        assert_eq!(
            affinities,
            vec![
                SourceAffinity::Agnostic,
                SourceAffinity::Agnostic,
                SourceAffinity::Only(Platform::Windows),
                SourceAffinity::Only(Platform::Linux),
                SourceAffinity::Agnostic,
            ]
        );
    }

    #[test]
    fn test_source_affinity_supports_matching_hosts_only() {
        assert!(SourceAffinity::Agnostic.supports(Platform::Linux));
        assert!(SourceAffinity::Agnostic.supports(Platform::Windows));
        assert!(SourceAffinity::Only(Platform::Linux).supports(Platform::Linux));
        assert!(!SourceAffinity::Only(Platform::Linux).supports(Platform::Windows));
        assert!(SourceAffinity::Only(Platform::Windows).supports(Platform::Windows));
        assert!(!SourceAffinity::Only(Platform::Windows).supports(Platform::Linux));
    }

    #[test]
    fn test_platform_names_round_trip() {
        for platform in [Platform::Linux, Platform::Windows] {
            assert_eq!(Platform::from_name(platform.name()), Some(platform));
        }
        assert_eq!(Platform::from_name("mac"), None);
        assert_eq!(Platform::from_name("win"), None);
        assert_eq!(Platform::from_name("Linux"), None);
        assert_eq!(SourceAffinity::Agnostic.name(), "any");
        assert_eq!(SourceAffinity::Only(Platform::Windows).name(), "windows");
    }

    #[test]
    fn test_platform_host_matches_the_build_target() {
        let expected = if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        };
        assert_eq!(Platform::host(), expected);
    }

    #[test]
    fn test_package_clone() {
        let pkg = Package::new("clone-test", "1.0.0");
        let cloned = pkg.clone();
        assert_eq!(pkg.name, cloned.name);
        assert_eq!(pkg.version, cloned.version);
    }
}
