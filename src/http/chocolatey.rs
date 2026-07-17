use serde::Deserialize;

use crate::core::package::{Package, PackageSource};
use crate::error::error::BallError;
use crate::http::HttpClient;

#[allow(dead_code)]
const CHOCOLATEY_FEED: &str = "https://community.chocolatey.org/api/v2";

#[derive(Debug, Deserialize)]
struct ODataResponse {
    d: ODataD,
}

#[derive(Debug, Deserialize)]
struct ODataD {
    results: Vec<ODataPackage>,
}

#[derive(Debug, Deserialize)]
struct ODataMetadata {
    #[serde(default, rename = "media_src")]
    media_src: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ODataPackage {
    id: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    authors: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    package_hash: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    package_hash_algorithm: Option<String>,
    #[serde(default)]
    dependencies: Option<String>,
    #[serde(default)]
    project_url: Option<String>,
    #[serde(default)]
    #[serde(rename = "__metadata")]
    metadata: Option<ODataMetadata>,
}

pub struct ChocolateyRegistry {
    client: HttpClient,
    feed_url: String,
}

impl ChocolateyRegistry {
    #[allow(dead_code)]
    pub fn new(client: HttpClient) -> Self {
        Self {
            client,
            feed_url: CHOCOLATEY_FEED.to_string(),
        }
    }

    pub fn with_feed_url(client: HttpClient, feed_url: String) -> Self {
        Self { client, feed_url }
    }

    pub fn fetch_package(&self, name: &str) -> Result<Package, BallError> {
        let url = format!(
            "{}/Packages()?$filter=Id eq '{}'&$orderby=Version desc&$top=1&$select=Id,Version,Description,Authors,PackageHash,PackageHashAlgorithm,ProjectUrl",
            self.feed_url.trim_end_matches('/'),
            name
        );

        let resp: ODataResponse = self
            .client
            .get_json_with_accept(&url, "application/json;odata=verbose")?;
        let entry = resp
            .d
            .results
            .into_iter()
            .next()
            .ok_or_else(|| BallError::PackageNotFound(name.to_string()))?;

        let version = normalize_nuget_version(&entry.version);
        let download_url = derive_download_url(&entry);
        let sha256 = entry.package_hash;

        Ok(Package {
            name: entry.id,
            version,
            description: entry.description,
            author: entry.authors,
            repository: entry.project_url,
            architectures: None,
            dependencies: parse_nuget_dependencies(&entry.dependencies),
            sha256,
            hash_algorithm: entry.package_hash_algorithm.map(|a| a.to_uppercase()),
            download_url,
            source: PackageSource::Chocolatey {
                feed_url: self.feed_url.clone(),
            },
        })
    }

    pub fn search(&self, query: &str) -> Result<Vec<Package>, BallError> {
        let url = format!(
            "{}/Packages()?$filter=substringof('{}',Id)&$orderby=DownloadCount desc&$top=20&$select=Id,Version,Description",
            self.feed_url.trim_end_matches('/'),
            query
        );

        let resp: ODataResponse = self
            .client
            .get_json_with_accept(&url, "application/json;odata=verbose")?;
        let packages: Vec<Package> = resp
            .d
            .results
            .into_iter()
            .map(|entry| Package {
                name: entry.id,
                version: normalize_nuget_version(&entry.version),
                description: entry.description,
                author: None,
                repository: None,
                architectures: None,
                dependencies: None,
                sha256: None,
                hash_algorithm: None,
                download_url: None,
                source: PackageSource::Chocolatey {
                    feed_url: self.feed_url.clone(),
                },
            })
            .collect();

        Ok(packages)
    }
}

fn normalize_nuget_version(raw: &str) -> String {
    let parts: Vec<&str> = raw.split('.').collect();
    if parts.len() == 4 && parts[3] == "0" {
        format!("{}.{}.{}", parts[0], parts[1], parts[2])
    } else {
        raw.to_string()
    }
}

fn parse_nuget_dependencies(raw: &Option<String>) -> Option<Vec<String>> {
    let deps = raw.as_ref()?;
    if deps.is_empty() {
        return None;
    }

    let entries: Vec<String> = deps
        .split('|')
        .filter_map(|group| {
            group.split(':').nth(1).map(|s| {
                let parts: Vec<&str> = s.split(':').collect();
                if parts.len() >= 2 {
                    format!(
                        "{}>={}",
                        parts[0],
                        parts[1].trim_start_matches('[').trim_end_matches(']')
                    )
                } else {
                    parts[0].to_string()
                }
            })
        })
        .collect();

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// Derive a Chocolatey download URL from an OData entry.
///
/// Prefers `__metadata.media_src` (the canonical download endpoint on the
/// Chocolatey OData v2 feed). Falls back to the structured
/// `https://community.chocolatey.org/api/package/{id}/{version}` URL, which
/// the API redirects to the same `media_src`.
fn derive_download_url(entry: &ODataPackage) -> Option<String> {
    if let Some(meta) = &entry.metadata {
        if let Some(media_src) = &meta.media_src {
            if !media_src.is_empty() {
                return Some(media_src.clone());
            }
        }
    }
    Some(format!(
        "https://community.chocolatey.org/api/package/{}/{}",
        entry.id, entry.version
    ))
}
