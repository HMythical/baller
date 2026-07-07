use serde::Deserialize;

use crate::core::package::{Package, PackageSource};
use crate::error::error::BallError;
use crate::http::HttpClient;

const GITHUB_API_BASE: &str = "https://api.github.com";

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    #[allow(dead_code)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    #[allow(dead_code)]
    content_type: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    size: Option<u64>,
}

pub struct GitHubRegistry {
    client: HttpClient,
}

impl GitHubRegistry {
    pub fn new(client: HttpClient) -> Self {
        Self { client }
    }

    pub fn fetch_package(&self, name: &str) -> Result<Package, BallError> {
        let (owner, repo) = parse_github_name(name)?;
        let url = format!(
            "{}/repos/{}/{}/releases/latest",
            GITHUB_API_BASE, owner, repo
        );
        let release: GitHubRelease = self.client.get_json(&url)?;

        let version = release.tag_name.trim_start_matches('v').to_string();
        let description = release.body.clone();
        let repository = Some(format!("https://github.com/{}/{}", owner, repo));
        let arch = detect_arch_string();

        let mut download_url = None;
        let sha256 = None;

        for asset in &release.assets {
            if asset.name.contains(&arch) {
                download_url = Some(asset.browser_download_url.clone());
                break;
            }
        }

        if download_url.is_none() {
            if let Some(first) = release.assets.first() {
                download_url = Some(first.browser_download_url.clone());
            }
        }

        Ok(Package {
            name: repo.clone(),
            version,
            description,
            author: None,
            repository,
            architectures: None,
            dependencies: None,
            sha256,
            download_url,
            source: PackageSource::GitHub {
                owner: owner.clone(),
                repo: repo.clone(),
            },
        })
    }

    #[allow(dead_code)]
    pub fn fetch_package_at_version(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Package, BallError> {
        let (owner, repo) = parse_github_name(name)?;
        let tag = if version.starts_with('v') {
            version.to_string()
        } else {
            format!("v{}", version)
        };
        let url = format!(
            "{}/repos/{}/{}/releases/tags/{}",
            GITHUB_API_BASE, owner, repo, tag
        );
        let release: GitHubRelease = self.client.get_json(&url)?;

        let version = release.tag_name.trim_start_matches('v').to_string();
        let description = release.body.clone();
        let repository = Some(format!("https://github.com/{}/{}", owner, repo));
        let arch = detect_arch_string();

        let mut download_url = None;
        let sha256 = None;

        for asset in &release.assets {
            if asset.name.contains(&arch) {
                download_url = Some(asset.browser_download_url.clone());
                break;
            }
        }

        if download_url.is_none() {
            if let Some(first) = release.assets.first() {
                download_url = Some(first.browser_download_url.clone());
            }
        }

        Ok(Package {
            name: repo.clone(),
            version,
            description,
            author: None,
            repository,
            architectures: None,
            dependencies: None,
            sha256,
            download_url,
            source: PackageSource::GitHub {
                owner: owner.clone(),
                repo: repo.clone(),
            },
        })
    }

    pub fn search(&self, query: &str) -> Result<Vec<Package>, BallError> {
        let url = format!(
            "{}/search/repositories?q={}+in:name&sort=stars&order=desc&per_page=10",
            GITHUB_API_BASE, query
        );
        let resp: GitHubSearchResponse = self.client.get_json(&url)?;

        let packages: Vec<Package> = resp
            .items
            .into_iter()
            .map(|item| {
                let name = item.name.clone();
                Package {
                    name,
                    version: "latest".to_string(),
                    description: item.description.clone(),
                    author: None,
                    repository: Some(item.html_url),
                    architectures: None,
                    dependencies: None,
                    sha256: None,
                    download_url: None,
                    source: PackageSource::GitHub {
                        owner: item.owner.login,
                        repo: item.name,
                    },
                }
            })
            .collect();

        Ok(packages)
    }
}

#[derive(Debug, Deserialize)]
struct GitHubSearchResponse {
    items: Vec<GitHubRepo>,
}

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    name: String,
    #[serde(default)]
    description: Option<String>,
    html_url: String,
    owner: GitHubOwner,
}

#[derive(Debug, Deserialize)]
struct GitHubOwner {
    login: String,
}

fn parse_github_name(name: &str) -> Result<(String, String), BallError> {
    let parts: Vec<&str> = name.split('/').collect();
    match parts.len() {
        1 => Ok(("HMythical".to_string(), parts[0].to_string())),
        2 => Ok((parts[0].to_string(), parts[1].to_string())),
        _ => Err(BallError::PackageNotFound(format!(
            "invalid GitHub package name '{}' — expected 'repo' or 'owner/repo'",
            name
        ))),
    }
}

fn detect_arch_string() -> String {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };

    format!("{}-{}", os, arch)
}
