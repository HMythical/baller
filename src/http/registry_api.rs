use crate::core::package::{Package, PackageSource};
use crate::error::error::BallError;
use crate::http::HttpClient;

pub struct BallerRegistryApi {
    client: HttpClient,
    base_url: String,
}

impl BallerRegistryApi {
    pub fn new(client: HttpClient, base_url: String) -> Self {
        Self { client, base_url }
    }

    pub fn fetch_package(&self, name: &str) -> Result<Package, BallError> {
        let url = format!("{}/packages/{}", self.base_url.trim_end_matches('/'), name);
        let mut pkg: Package = self.client.get_json(&url)?;
        pkg.source = PackageSource::BallerRegistry {
            url: self.base_url.clone(),
        };
        Ok(pkg)
    }

    #[allow(dead_code)]
    pub fn fetch_package_at_version(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Package, BallError> {
        let url = format!(
            "{}/packages/{}/versions/{}",
            self.base_url.trim_end_matches('/'),
            name,
            version
        );
        let mut pkg: Package = self.client.get_json(&url)?;
        pkg.source = PackageSource::BallerRegistry {
            url: self.base_url.clone(),
        };
        Ok(pkg)
    }

    pub fn search(&self, query: &str) -> Result<Vec<Package>, BallError> {
        let url = format!("{}/search?q={}", self.base_url.trim_end_matches('/'), query);
        let results: Vec<Package> = self.client.get_json(&url)?;
        Ok(results)
    }

    #[allow(dead_code)]
    pub fn resolve_latest(&self, name: &str) -> Result<String, BallError> {
        let url = format!(
            "{}/packages/{}/latest",
            self.base_url.trim_end_matches('/'),
            name
        );
        #[derive(serde::Deserialize)]
        struct LatestResponse {
            version: String,
        }
        let resp: LatestResponse = self.client.get_json(&url)?;
        Ok(resp.version)
    }
}
