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
        let github = GitHubRegistry::new(client.clone());
        let baller_api = BallerRegistryApi::new(
            client.clone(),
            "https://registry.baller.dev/api".to_string(),
        );
        let chocolatey = ChocolateyRegistry::new(client);
        let system = SystemRegistry::detect();

        Self {
            github,
            baller_api,
            chocolatey,
            system,
            source_order: vec![
                RegistrySource::GitHub,
                RegistrySource::BallerRegistry,
                RegistrySource::Chocolatey,
            ],
        }
    }

    pub fn with_source_order(
        client: HttpClient,
        source_order: Vec<RegistrySource>,
        baller_registry_url: String,
        chocolatey_feed_url: String,
    ) -> Self {
        let github = GitHubRegistry::new(client.clone());
        let baller_api = BallerRegistryApi::new(client.clone(), baller_registry_url);
        let chocolatey = ChocolateyRegistry::with_feed_url(client, chocolatey_feed_url);
        let system = SystemRegistry::detect();

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

        Err(if errors.is_empty() {
            BallError::PackageNotFound(format!("{} not found in any configured registry", name))
        } else {
            BallError::PackageNotFound(format!(
                "{} not found. Sources tried:\n  {}",
                name,
                errors.join("\n  ")
            ))
        })
    }

    #[allow(dead_code)]
    pub fn fetch_package_from_source(
        &self,
        source: &RegistrySource,
        name: &str,
    ) -> Result<Package, BallError> {
        self.try_fetch(source, name)
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
    fn test_registry_index_trait_object() {
        // Just verify the trait is object-safe
        fn _take_ref(_: &dyn RegistryIndex) {}
        let _ = _take_ref;
    }
}
