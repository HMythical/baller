use serde::Deserialize;

use crate::core::dep_solver::parse_dependency_line;
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

    /// Fetch one exact version from the feed.
    ///
    /// NuGet stores four-part versions (`1.2.3.0`), so a three-part request is
    /// retried with the `.0` suffix before giving up.
    pub fn fetch_package_at_version(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Package, BallError> {
        let candidates = version_candidates(version);

        let mut last_err = None;
        for candidate in &candidates {
            match self.fetch_exact_version(name, candidate) {
                Ok(pkg) => return Ok(pkg),
                Err(e) => last_err = Some(e),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            BallError::PackageNotFound(format!("{} version {} not found", name, version))
        }))
    }

    fn fetch_exact_version(&self, name: &str, version: &str) -> Result<Package, BallError> {
        let url = format!(
            "{}/Packages()?$filter=Id eq '{}' and Version eq '{}'&$top=1&$select=Id,Version,Description,Authors,PackageHash,PackageHashAlgorithm,ProjectUrl",
            self.feed_url.trim_end_matches('/'),
            name,
            version
        );

        let resp: ODataResponse = self
            .client
            .get_json_with_accept(&url, "application/json;odata=verbose")?;
        let entry = resp.d.results.into_iter().next().ok_or_else(|| {
            BallError::PackageNotFound(format!("{} version {} not found", name, version))
        })?;

        let normalized = normalize_nuget_version(&entry.version);
        let download_url = derive_download_url(&entry);
        let sha256 = entry.package_hash;

        Ok(Package {
            name: entry.id,
            version: normalized,
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

/// The version strings to try against the feed, in order.
///
/// NuGet stores some versions with a fourth `.0` segment that
/// [`normalize_nuget_version`] strips on the way out, so a three-part request
/// gets a four-part retry.
fn version_candidates(version: &str) -> Vec<String> {
    let mut candidates = vec![version.to_string()];
    if version.split('.').count() == 3 {
        candidates.push(format!("{}.0", version));
    }
    candidates
}

fn normalize_nuget_version(raw: &str) -> String {
    let parts: Vec<&str> = raw.split('.').collect();
    if parts.len() == 4 && parts[3] == "0" {
        format!("{}.{}.{}", parts[0], parts[1], parts[2])
    } else {
        raw.to_string()
    }
}

/// Translate a NuGet `Dependencies` string (`id:range:framework|…`) into
/// baller dependency lines (`id >=min`).
fn parse_nuget_dependencies(raw: &Option<String>) -> Option<Vec<String>> {
    let deps = raw.as_ref()?;
    let entries: Vec<String> = deps.split('|').filter_map(nuget_dependency_line).collect();

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// One `id:range:framework` group as a dependency line.
///
/// Only the range's lower bound is kept (`[1.0, 2.0)` → `>=1.0`, `(1.0,)` →
/// `>1.0`): Chocolatey always serves its latest version, so an upper bound
/// could only fail. A bound semver cannot express, such as a four-part NuGet
/// version, is dropped rather than emitted as a line that fails resolution.
fn nuget_dependency_line(group: &str) -> Option<String> {
    let mut fields = group.split(':');
    let id = fields.next()?.trim();
    if id.is_empty() {
        return None;
    }

    let range = fields.next().unwrap_or("").trim();
    let op = if range.starts_with('(') { ">" } else { ">=" };
    let min = range
        .trim_start_matches(['[', '('])
        .split(',')
        .next()
        .unwrap_or("")
        .trim_end_matches([']', ')'])
        .trim();
    if min.is_empty() {
        return Some(id.to_string());
    }

    let line = format!("{} {}{}", id, op, normalize_nuget_version(min));
    if parse_dependency_line(&line).is_err() {
        tracing::debug!(
            "chocolatey dependency '{}': dropping version range '{}' that semver cannot express",
            id,
            range
        );
        return Some(id.to_string());
    }
    Some(line)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_candidates_three_part_gets_nuget_retry() {
        assert_eq!(
            version_candidates("14.1.0"),
            vec!["14.1.0".to_string(), "14.1.0.0".to_string()]
        );
    }

    #[test]
    fn test_version_candidates_four_part_used_as_is() {
        assert_eq!(version_candidates("14.1.0.0"), vec!["14.1.0.0".to_string()]);
    }

    #[test]
    fn test_version_candidates_prerelease_used_as_is() {
        assert_eq!(
            version_candidates("1.0.0-beta"),
            vec!["1.0.0-beta".to_string(), "1.0.0-beta.0".to_string()]
        );
    }

    #[test]
    fn test_normalize_nuget_version_strips_trailing_zero() {
        assert_eq!(normalize_nuget_version("14.1.0.0"), "14.1.0");
        assert_eq!(normalize_nuget_version("14.1.0"), "14.1.0");
        assert_eq!(normalize_nuget_version("14.1.0.3"), "14.1.0.3");
    }

    fn deps(raw: &str) -> Option<Vec<String>> {
        parse_nuget_dependencies(&Some(raw.to_string()))
    }

    #[test]
    fn test_parse_nuget_dependencies_returns_package_ids() {
        // Previously this returned the version ranges ["1.3.3", "14.0"] as names
        let parsed = deps("chocolatey-core.extension:1.3.3:|vcredist140:14.0:").unwrap();
        assert_eq!(
            parsed,
            vec![
                "chocolatey-core.extension >=1.3.3".to_string(),
                "vcredist140 >=14.0".to_string()
            ]
        );

        for line in &parsed {
            let dep = parse_dependency_line(line).unwrap();
            assert!(!dep.name.chars().next().unwrap().is_ascii_digit());
        }
    }

    #[test]
    fn test_parse_nuget_dependencies_interval_ranges() {
        assert_eq!(deps("a:[1.0, 2.0):").unwrap(), vec!["a >=1.0"]);
        assert_eq!(deps("a:(1.0,):").unwrap(), vec!["a >1.0"]);
        assert_eq!(deps("a:[1.0]:").unwrap(), vec!["a >=1.0"]);
        assert_eq!(deps("a:(,2.0]:").unwrap(), vec!["a"]);
        assert_eq!(deps("a::").unwrap(), vec!["a"]);
        assert_eq!(deps("a").unwrap(), vec!["a"]);
    }

    #[test]
    fn test_parse_nuget_dependencies_four_part_versions() {
        assert_eq!(deps("a:14.1.0.0:").unwrap(), vec!["a >=14.1.0"]);
        // Not expressible in semver: the bound is dropped, the dependency kept
        assert_eq!(
            deps("vcredist140:14.16.27012.6:").unwrap(),
            vec!["vcredist140"]
        );
    }

    #[test]
    fn test_parse_nuget_dependencies_empty_input() {
        assert_eq!(parse_nuget_dependencies(&None), None);
        assert_eq!(deps(""), None);
        assert_eq!(deps("|"), None);
        assert_eq!(deps(":1.0:"), None);
    }

    fn odata_entry(media_src: Option<&str>, with_metadata: bool) -> ODataPackage {
        ODataPackage {
            id: "7zip".to_string(),
            version: "24.8.0".to_string(),
            description: None,
            authors: None,
            download_url: None,
            package_hash: None,
            package_hash_algorithm: None,
            dependencies: None,
            project_url: None,
            metadata: with_metadata.then(|| ODataMetadata {
                media_src: media_src.map(str::to_string),
            }),
        }
    }

    #[test]
    fn test_derive_download_url_prefers_media_src() {
        let entry = odata_entry(Some("https://cdn.example/7zip.24.8.0.nupkg"), true);
        assert_eq!(
            derive_download_url(&entry).as_deref(),
            Some("https://cdn.example/7zip.24.8.0.nupkg")
        );
    }

    #[test]
    fn test_derive_download_url_falls_back_to_the_package_endpoint() {
        let expected = Some("https://community.chocolatey.org/api/package/7zip/24.8.0");
        for entry in [
            odata_entry(Some(""), true),
            odata_entry(None, true),
            odata_entry(None, false),
        ] {
            assert_eq!(derive_download_url(&entry).as_deref(), expected);
        }
    }
}
