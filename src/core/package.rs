use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum PackageSource {
    GitHub { owner: String, repo: String },
    BallerRegistry { url: String },
    Chocolatey { feed_url: String },
    System { manager: String },
}

impl Default for PackageSource {
    fn default() -> Self {
        PackageSource::GitHub {
            owner: String::new(),
            repo: String::new(),
        }
    }
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

    pub download_url: Option<String>,

    #[serde(default)]
    pub source: PackageSource,
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
            download_url: None,
            source: PackageSource::GitHub {
                owner: String::new(),
                repo: name.to_string(),
            },
        }
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
            download_url: Some("https://example.com/pkg.tar.gz".to_string()),
            source: PackageSource::BallerRegistry {
                url: "https://reg.example.com".to_string(),
            },
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

    #[test]
    fn test_package_clone() {
        let pkg = Package::new("clone-test", "1.0.0");
        let cloned = pkg.clone();
        assert_eq!(pkg.name, cloned.name);
        assert_eq!(pkg.version, cloned.version);
    }
}
