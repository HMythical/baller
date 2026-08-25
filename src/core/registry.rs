use crate::core::package::Package;
use crate::error::error::BallError;
use crate::http::chocolatey::ChocolateyRegistry;
use crate::http::github::GitHubRegistry;
use crate::http::registry_api::BallerRegistryApi;
use crate::http::system::SystemRegistry;
use crate::http::HttpClient;

#[derive(Debug, Clone, PartialEq)]
pub enum RegistrySource {
    GitHub,
    BallerRegistry,
    Chocolatey,
    System,
}

impl RegistrySource {
    /// The name this source is written as in `baller.conf`'s `source_order`
    pub fn config_name(&self) -> &'static str {
        match self {
            RegistrySource::GitHub => "github",
            RegistrySource::BallerRegistry => "baller",
            RegistrySource::Chocolatey => "chocolatey",
            RegistrySource::System => "system",
        }
    }

    /// The value stored in the database's `source` column for this source
    pub fn db_name(&self) -> &'static str {
        match self {
            RegistrySource::GitHub => "github",
            RegistrySource::BallerRegistry => "baller_registry",
            RegistrySource::Chocolatey => "chocolatey",
            RegistrySource::System => "system",
        }
    }

    /// Parse a `source_order` entry; unknown names are ignored by the caller
    pub fn from_config_name(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "github" => Some(RegistrySource::GitHub),
            "baller" => Some(RegistrySource::BallerRegistry),
            "chocolatey" => Some(RegistrySource::Chocolatey),
            "system" => Some(RegistrySource::System),
            _ => None,
        }
    }
}

/// The source chain this platform prefers: the Baller registry first, then the
/// native ecosystem (Chocolatey on Windows, the distro package manager on
/// Linux), with GitHub as the last-resort fallback.
pub fn default_source_order() -> Vec<RegistrySource> {
    if cfg!(target_os = "windows") {
        vec![
            RegistrySource::BallerRegistry,
            RegistrySource::Chocolatey,
            RegistrySource::GitHub,
        ]
    } else {
        vec![
            RegistrySource::BallerRegistry,
            RegistrySource::System,
            RegistrySource::GitHub,
        ]
    }
}

#[cfg(target_os = "linux")]
fn system_registry() -> SystemRegistry {
    SystemRegistry::detect()
}

/// Windows has no `/etc/os-release`, so detection is skipped entirely there
#[cfg(not(target_os = "linux"))]
fn system_registry() -> SystemRegistry {
    SystemRegistry::unavailable()
}

#[allow(dead_code)]
pub trait RegistryIndex {
    fn fetch_package(&self, name: &str) -> Result<Package, BallError>;
    fn search(&self, query: &str) -> Result<Vec<Package>, BallError>;
}

pub struct RegistryClient {
    github: GitHubRegistry,
    baller_api: BallerRegistryApi,
    chocolatey: ChocolateyRegistry,
    system: SystemRegistry,
    source_order: Vec<RegistrySource>,
}

impl RegistryClient {
    #[allow(dead_code)]
    pub fn new(client: HttpClient) -> Self {
        let github = GitHubRegistry::new(client.clone(), None);
        let baller_api = BallerRegistryApi::new(
            client.clone(),
            "https://registry.baller.dev/api".to_string(),
        );
        let chocolatey = ChocolateyRegistry::new(client);
        let system = system_registry();

        Self {
            github,
            baller_api,
            chocolatey,
            system,
            source_order: default_source_order(),
        }
    }

    pub fn with_source_order(
        client: HttpClient,
        source_order: Vec<RegistrySource>,
        baller_registry_url: String,
        chocolatey_feed_url: String,
        github_default_owner: Option<String>,
    ) -> Self {
        let github = GitHubRegistry::new(client.clone(), github_default_owner);
        let baller_api = BallerRegistryApi::new(client.clone(), baller_registry_url);
        let chocolatey = ChocolateyRegistry::with_feed_url(client, chocolatey_feed_url);
        let system = system_registry();

        Self {
            github,
            baller_api,
            chocolatey,
            system,
            source_order,
        }
    }

    pub fn fetch_package(&self, name: &str) -> Result<Package, BallError> {
        let mut errors: Vec<String> = Vec::new();

        for source in &self.source_order {
            match self.try_fetch(source, name) {
                Ok(pkg) => return Ok(pkg),
                Err(e) => {
                    errors.push(format!("{:?}: {}", source, e));
                }
            }
        }

        Err(not_found(name, errors))
    }

    /// Fetch a specific version, walking the same source chain.
    ///
    /// Only GitHub and Chocolatey can pin; the other sources contribute a
    /// clear reason to the aggregated error.
    pub fn fetch_package_at_version(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Package, BallError> {
        let mut errors: Vec<String> = Vec::new();

        for source in &self.source_order {
            match self.try_fetch_at_version(source, name, version) {
                Ok(pkg) => return Ok(pkg),
                Err(e) => {
                    errors.push(format!("{:?}: {}", source, e));
                }
            }
        }

        Err(not_found(&format!("{}@{}", name, version), errors))
    }

    pub fn fetch_package_from_source(
        &self,
        source: &RegistrySource,
        name: &str,
    ) -> Result<Package, BallError> {
        self.try_fetch(source, name)
    }

    pub fn fetch_package_at_version_from_source(
        &self,
        source: &RegistrySource,
        name: &str,
        version: &str,
    ) -> Result<Package, BallError> {
        self.try_fetch_at_version(source, name, version)
    }

    /// The native package manager detected for this host, if any
    pub fn system_manager_name(&self) -> Option<&'static str> {
        self.system.manager_name()
    }

    pub fn search(&self, query: &str) -> Result<Vec<Package>, BallError> {
        let mut all_results = Vec::new();

        for source in &self.source_order {
            let results = match source {
                RegistrySource::GitHub => self.github.search(query),
                RegistrySource::BallerRegistry => self.baller_api.search(query),
                RegistrySource::Chocolatey => self.chocolatey.search(query),
                RegistrySource::System => self.system.search(query),
            };

            match results {
                Ok(pkgs) => all_results.extend(pkgs),
                Err(_) => continue,
            }

            if all_results.len() >= 20 {
                break;
            }
        }

        Ok(all_results)
    }

    fn try_fetch(&self, source: &RegistrySource, name: &str) -> Result<Package, BallError> {
        match source {
            RegistrySource::GitHub => self.github.fetch_package(name),
            RegistrySource::BallerRegistry => self.baller_api.fetch_package(name),
            RegistrySource::Chocolatey => self.chocolatey.fetch_package(name),
            RegistrySource::System => self.system.fetch_package(name),
        }
    }

    fn try_fetch_at_version(
        &self,
        source: &RegistrySource,
        name: &str,
        version: &str,
    ) -> Result<Package, BallError> {
        match source {
            RegistrySource::GitHub => self.github.fetch_package_at_version(name, version),
            RegistrySource::Chocolatey => self.chocolatey.fetch_package_at_version(name, version),
            RegistrySource::BallerRegistry => Err(BallError::UnsupportedCommand(
                "version pinning against the Baller registry is not supported yet".to_string(),
            )),
            RegistrySource::System => Err(BallError::PackageManagerError(format!(
                "system packages always install the latest available version — cannot pin '{}'",
                name
            ))),
        }
    }
}

fn not_found(name: &str, errors: Vec<String>) -> BallError {
    if errors.is_empty() {
        BallError::PackageNotFound(format!("{} not found in any configured registry", name))
    } else {
        BallError::PackageNotFound(format!(
            "{} not found. Sources tried:\n  {}",
            name,
            errors.join("\n  ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_source_debug() {
        let s = RegistrySource::GitHub;
        let d = format!("{:?}", s);
        assert_eq!(d, "GitHub");
    }

    #[test]
    fn test_registry_source_clone() {
        let a = RegistrySource::BallerRegistry;
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn test_registry_source_equality() {
        assert_eq!(RegistrySource::GitHub, RegistrySource::GitHub);
        assert_ne!(RegistrySource::GitHub, RegistrySource::Chocolatey);
    }

    #[test]
    fn test_default_source_order_prefers_platform_native() {
        let order = default_source_order();

        assert_eq!(order.first(), Some(&RegistrySource::BallerRegistry));
        assert_eq!(order.last(), Some(&RegistrySource::GitHub));

        if cfg!(target_os = "windows") {
            assert_eq!(
                order,
                vec![
                    RegistrySource::BallerRegistry,
                    RegistrySource::Chocolatey,
                    RegistrySource::GitHub
                ]
            );
            assert!(!order.contains(&RegistrySource::System));
        } else {
            assert_eq!(
                order,
                vec![
                    RegistrySource::BallerRegistry,
                    RegistrySource::System,
                    RegistrySource::GitHub
                ]
            );
            assert!(!order.contains(&RegistrySource::Chocolatey));
        }
    }

    #[test]
    fn test_config_name_round_trip() {
        for source in [
            RegistrySource::GitHub,
            RegistrySource::BallerRegistry,
            RegistrySource::Chocolatey,
            RegistrySource::System,
        ] {
            let name = source.config_name();
            assert_eq!(RegistrySource::from_config_name(name), Some(source));
        }
    }

    #[test]
    fn test_from_config_name_is_case_insensitive() {
        assert_eq!(
            RegistrySource::from_config_name(" GitHub "),
            Some(RegistrySource::GitHub)
        );
    }

    #[test]
    fn test_from_config_name_unknown() {
        assert_eq!(RegistrySource::from_config_name("npm"), None);
    }

    #[test]
    fn test_registry_index_trait_object() {
        // Just verify the trait is object-safe
        fn _take_ref(_: &dyn RegistryIndex) {}
        let _ = _take_ref;
    }
}
