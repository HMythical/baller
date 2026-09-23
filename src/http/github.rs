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
    default_owner: Option<String>,
}

impl GitHubRegistry {
    pub fn new(client: HttpClient, default_owner: Option<String>) -> Self {
        Self {
            client,
            default_owner,
        }
    }

    pub fn fetch_package(&self, name: &str) -> Result<Package, BallError> {
        let (owner, repo) = parse_github_name(name, &self.default_owner)?;
        let url = format!(
            "{}/repos/{}/{}/releases/latest",
            GITHUB_API_BASE, owner, repo
        );
        let release: GitHubRelease = self.client.get_json(&url)?;

        let version = release.tag_name.trim_start_matches('v').to_string();
        let description = release.body.clone();
        let repository = Some(format!("https://github.com/{}/{}", owner, repo));
        let sha256 = None;

        let asset = select_asset(&release.assets, &host_platform(), &repo)?;
        tracing::debug!(
            "github: selected asset '{}' for {}/{} v{}",
            asset.name,
            owner,
            repo,
            version
        );
        let download_url = Some(asset.browser_download_url.clone());

        Ok(Package {
            name: repo.clone(),
            version,
            description,
            author: None,
            repository,
            architectures: None,
            dependencies: None,
            sha256,
            hash_algorithm: None,
            download_url,
            source: PackageSource::GitHub {
                owner: owner.clone(),
                repo: repo.clone(),
            },
            advisory: None,
            vulnerabilities: Vec::new(),
        })
    }

    #[allow(dead_code)]
    pub fn fetch_package_at_version(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Package, BallError> {
        let (owner, repo) = parse_github_name(name, &self.default_owner)?;
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
        let sha256 = None;

        let asset = select_asset(&release.assets, &host_platform(), &repo)?;
        tracing::debug!(
            "github: selected asset '{}' for {}/{} v{}",
            asset.name,
            owner,
            repo,
            version
        );
        let download_url = Some(asset.browser_download_url.clone());

        Ok(Package {
            name: repo.clone(),
            version,
            description,
            author: None,
            repository,
            architectures: None,
            dependencies: None,
            sha256,
            hash_algorithm: None,
            download_url,
            source: PackageSource::GitHub {
                owner: owner.clone(),
                repo: repo.clone(),
            },
            advisory: None,
            vulnerabilities: Vec::new(),
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
                    hash_algorithm: None,
                    download_url: None,
                    source: PackageSource::GitHub {
                        owner: item.owner.login,
                        repo: item.name,
                    },
                    advisory: None,
                    vulnerabilities: Vec::new(),
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

fn parse_github_name(
    name: &str,
    default_owner: &Option<String>,
) -> Result<(String, String), BallError> {
    let parts: Vec<&str> = name.split('/').collect();
    match parts.len() {
        2 => Ok((parts[0].to_string(), parts[1].to_string())),
        1 => match default_owner {
            None => Err(BallError::PackageNotFound(format!(
                "invalid GitHub package name '{}' — expected 'owner/repo'",
                name
            ))),
            Some(owner) => Ok((owner.to_string(), name.to_string())),
        },
        _ => Err(BallError::PackageNotFound(format!(
            "invalid GitHub package name '{}' — expected 'owner/repo'",
            name
        ))),
    }
}

/// The host OS/arch pair an asset must match.
///
/// Kept as plain strings so the matcher stays pure and unit-testable: the
/// `cfg!` lookups happen once in [`host_platform`], never inside the matcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlatformSpec {
    os: &'static str,
    arch: &'static str,
}

/// The platform this build of baller runs on.
///
/// Unknown targets fall back to `linux`/`x86_64`, matching the rest of the
/// codebase's Linux-or-Windows assumption.
fn host_platform() -> PlatformSpec {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };

    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };

    PlatformSpec { os, arch }
}

/// Asset-name fragments that identify a build for `spec`, most specific first.
///
/// Release naming is not standardized, so the list covers the conventions in
/// the wild: Rust target triples, Go's `os_arch`, and bare architecture
/// tokens. Order matters — a gnu triple is preferred over musl, and full
/// triples are preferred over bare tokens, so the closest build wins when a
/// release ships several.
fn candidate_tags(spec: &PlatformSpec) -> Vec<&'static str> {
    match (spec.os, spec.arch) {
        ("linux", "x86_64") => vec![
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            "x86_64-linux-gnu",
            "x86_64-linux-musl",
            "amd64-unknown-linux-gnu",
            "linux_amd64",
            "linux-amd64",
            "linux.amd64",
            "linux_x86_64",
            "linux-x86_64",
            "linux.x86_64",
            "linux64",
            "x86_64-linux",
            "amd64-linux",
            "x86_64",
            "x86-64",
            "amd64",
        ],
        ("linux", "arm64") => vec![
            "aarch64-unknown-linux-gnu",
            "aarch64-unknown-linux-musl",
            "aarch64-linux-gnu",
            "aarch64-linux-musl",
            "linux_arm64",
            "linux-arm64",
            "linux.arm64",
            "linux_aarch64",
            "linux-aarch64",
            "linux.aarch64",
            "aarch64-linux",
            "arm64-linux",
            "aarch64",
            "arm64",
        ],
        ("windows", "x86_64") => vec![
            "x86_64-pc-windows-msvc",
            "x86_64-pc-windows-gnu",
            "x86_64-windows-msvc",
            "x86_64-windows-gnu",
            "windows_amd64",
            "windows-amd64",
            "windows.amd64",
            "windows_x86_64",
            "windows-x86_64",
            "windows.x86_64",
            "win64",
            "windows64",
            "x86_64-windows",
            "amd64-windows",
            "x86_64",
            "x86-64",
            "amd64",
        ],
        ("windows", "arm64") => vec![
            "aarch64-pc-windows-msvc",
            "aarch64-pc-windows-gnu",
            "windows_arm64",
            "windows-arm64",
            "windows.arm64",
            "windows_aarch64",
            "windows-aarch64",
            "arm64-windows",
            "aarch64-windows",
            "aarch64",
            "arm64",
        ],
        _ => vec!["x86_64", "amd64"],
    }
}

/// Suffixes that are never an installable program: packages for another
/// package manager, signatures, checksums and release notes.
const FORBIDDEN_EXTENSIONS: &[&str] = &[
    ".deb",
    ".rpm",
    ".apk",
    ".pkg",
    ".msi",
    ".sig",
    ".asc",
    ".pem",
    ".sha1",
    ".sha256",
    ".sha512",
    ".sum",
    ".sbom",
    ".txt",
    ".json",
    ".yml",
    ".yaml",
    ".dsc",
    ".buildinfo",
    ".changes",
];

/// OS tokens that rule an asset out, keyed by the host OS they are foreign to.
const FOREIGN_OS_TOKENS: &[&str] = &[
    "darwin",
    "macos",
    "mac-os",
    "apple",
    "osx",
    "android",
    "freebsd",
    "openbsd",
    "netbsd",
    "dragonfly",
    "solaris",
    "illumos",
    "ios",
    "wasm",
    "wasi",
];

/// Architecture tokens, grouped so the wrong-arch set is the complement of the
/// host's own group.
const AMD64_TOKENS: &[&str] = &["x86_64", "x86-64", "amd64"];
const ARM64_TOKENS: &[&str] = &["aarch64", "arm64", "armv7", "armv8", "armhf", "armv6"];
const OTHER_ARCH_TOKENS: &[&str] = &[
    "riscv", "ppc64", "powerpc", "s390x", "mips", "i386", "i686", "386", "x86_32", "win32",
];

/// Extensions `Downloader::extract_archive` knows how to unpack.
///
/// Releases often ship the same build twice — a bare binary and an archive of
/// it — and only the archive can be extracted, so a match on one of these wins
/// a tie against a match with no recognized extension.
const ARCHIVE_EXTENSIONS: &[&str] = &[
    ".tar.gz", ".tgz", ".tar.bz2", ".tbz2", ".tar.xz", ".txz", ".tar", ".zip", ".nupkg", ".gz",
];

fn is_known_archive(name: &str) -> bool {
    ARCHIVE_EXTENSIONS.iter().any(|ext| name.ends_with(ext))
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

/// Whether `name` is disqualified for `spec` regardless of what it matches.
///
/// Exclusion runs before matching so a `checksums.txt` or a `darwin-amd64`
/// tarball can never win on a bare `amd64` token.
fn is_excluded(name: &str, spec: &PlatformSpec) -> bool {
    if FORBIDDEN_EXTENSIONS.iter().any(|ext| name.ends_with(ext)) {
        return true;
    }

    if contains_any(name, FOREIGN_OS_TOKENS) {
        return true;
    }

    let foreign_os = if spec.os == "linux" {
        "windows"
    } else {
        "linux"
    };
    if name.contains(foreign_os) || (spec.os == "linux" && name.contains("win64")) {
        return true;
    }

    let (host_arch_tokens, foreign_arch_tokens) = if spec.arch == "x86_64" {
        (AMD64_TOKENS, ARM64_TOKENS)
    } else {
        (ARM64_TOKENS, AMD64_TOKENS)
    };

    if contains_any(name, foreign_arch_tokens) && !contains_any(name, host_arch_tokens) {
        return true;
    }

    if contains_any(name, OTHER_ARCH_TOKENS) && !contains_any(name, host_arch_tokens) {
        return true;
    }

    false
}

/// Pick the release asset built for `spec`.
///
/// Assets are excluded first (foreign OS/arch, distro packages, checksums,
/// signatures), then the surviving names are scanned in [`candidate_tags`]
/// order so the most specific naming convention wins. There is deliberately no
/// "first asset" fallback: an arbitrary asset installs something that cannot
/// run on this host, which is what issue #13 reported.
fn select_asset<'a>(
    assets: &'a [GitHubAsset],
    spec: &PlatformSpec,
    package: &str,
) -> Result<&'a GitHubAsset, BallError> {
    let candidates: Vec<(&GitHubAsset, String)> = assets
        .iter()
        .map(|asset| (asset, asset.name.to_lowercase()))
        .filter(|(_, lowered)| !is_excluded(lowered, spec))
        .collect();

    for tag in candidate_tags(spec) {
        let mut tagged = candidates
            .iter()
            .filter(|(_, lowered)| lowered.contains(tag));

        let first = match tagged.next() {
            Some(hit) => hit,
            None => continue,
        };

        if is_known_archive(&first.1) {
            return Ok(first.0);
        }

        // The first hit is something like a bare `yq_linux_amd64` ELF; take an
        // extractable archive of the same build if the release ships one.
        return Ok(tagged
            .find(|(_, lowered)| is_known_archive(lowered))
            .unwrap_or(first)
            .0);
    }

    Err(BallError::NoMatchingAsset {
        package: package.to_string(),
        platform: format!("{}-{}", spec.os, spec.arch),
        assets: assets.iter().map(|asset| asset.name.clone()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX_X64: PlatformSpec = PlatformSpec {
        os: "linux",
        arch: "x86_64",
    };
    const LINUX_ARM64: PlatformSpec = PlatformSpec {
        os: "linux",
        arch: "arm64",
    };
    const WINDOWS_X64: PlatformSpec = PlatformSpec {
        os: "windows",
        arch: "x86_64",
    };

    fn assets(names: &[&str]) -> Vec<GitHubAsset> {
        names
            .iter()
            .map(|name| GitHubAsset {
                name: (*name).to_string(),
                browser_download_url: format!("https://example.test/{}", name),
                content_type: None,
                size: None,
            })
            .collect()
    }

    fn pick<'a>(list: &'a [GitHubAsset], spec: &PlatformSpec) -> &'a str {
        &select_asset(list, spec, "pkg")
            .expect("an asset should match")
            .name
    }

    #[test]
    fn test_ripgrep_prefers_the_linux_gnu_triple() {
        let list = assets(&[
            "ripgrep-14.1.1-aarch64-apple-darwin.tar.gz",
            "ripgrep-14.1.1-aarch64-unknown-linux-gnu.tar.gz",
            "ripgrep-14.1.1-x86_64-apple-darwin.tar.gz",
            "ripgrep-14.1.1-x86_64-pc-windows-msvc.zip",
            "ripgrep-14.1.1-x86_64-unknown-linux-musl.tar.gz",
            "ripgrep_14.1.1-1_amd64.deb",
            "ripgrep-14.1.1.tar.gz",
        ]);
        assert_eq!(
            pick(&list, &LINUX_X64),
            "ripgrep-14.1.1-x86_64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            pick(&list, &LINUX_ARM64),
            "ripgrep-14.1.1-aarch64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            pick(&list, &WINDOWS_X64),
            "ripgrep-14.1.1-x86_64-pc-windows-msvc.zip"
        );
    }

    #[test]
    fn test_gnu_wins_over_musl_when_both_ship() {
        let list = assets(&[
            "tool-x86_64-unknown-linux-musl.tar.gz",
            "tool-x86_64-unknown-linux-gnu.tar.gz",
        ]);
        assert_eq!(
            pick(&list, &LINUX_X64),
            "tool-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn test_fzf_go_style_names() {
        let list = assets(&[
            "fzf-0.55.0-android_arm64.tar.gz",
            "fzf-0.55.0-darwin_amd64.tar.gz",
            "fzf-0.55.0-linux_amd64.tar.gz",
            "fzf-0.55.0-linux_arm64.tar.gz",
            "fzf-0.55.0-windows_amd64.zip",
        ]);
        assert_eq!(pick(&list, &LINUX_X64), "fzf-0.55.0-linux_amd64.tar.gz");
        assert_eq!(pick(&list, &LINUX_ARM64), "fzf-0.55.0-linux_arm64.tar.gz");
        assert_eq!(pick(&list, &WINDOWS_X64), "fzf-0.55.0-windows_amd64.zip");
    }

    #[test]
    fn test_archive_beats_a_bare_binary_of_the_same_build() {
        let list = assets(&[
            "yq_linux_amd64",
            "yq_linux_amd64.tar.gz",
            "yq_linux_arm64.tar.gz",
        ]);
        assert_eq!(pick(&list, &LINUX_X64), "yq_linux_amd64.tar.gz");
    }

    #[test]
    fn test_bare_binary_is_still_chosen_when_it_is_the_only_build() {
        let list = assets(&["tool_linux_amd64", "tool_darwin_amd64"]);
        assert_eq!(pick(&list, &LINUX_X64), "tool_linux_amd64");
    }

    #[test]
    fn test_checksums_and_distro_packages_never_win() {
        let list = assets(&[
            "checksums.txt",
            "tool_1.0.0_amd64.deb",
            "tool-1.0.0-1.x86_64.rpm",
            "tool-linux-amd64.tar.gz.sha256",
            "tool-linux-amd64.tar.gz.sig",
            "tool-linux-amd64.tar.gz",
        ]);
        assert_eq!(pick(&list, &LINUX_X64), "tool-linux-amd64.tar.gz");
    }

    #[test]
    fn test_source_tarball_is_not_mistaken_for_a_build() {
        let list = assets(&["tool-1.0.0-src.tar.gz", "tool-1.0.0.tar.gz"]);
        let err = select_asset(&list, &LINUX_X64, "tool").expect_err("no host build");
        let msg = format!("{}", err);
        assert!(msg.contains("no linux-x86_64 asset for 'tool'"));
        assert!(msg.contains("tool-1.0.0.tar.gz"));
    }

    #[test]
    fn test_macos_only_release_errors_with_the_available_names() {
        let list = assets(&[
            "tool-x86_64-apple-darwin.tar.gz",
            "tool-aarch64-apple-darwin.tar.gz",
        ]);
        let err = select_asset(&list, &LINUX_X64, "tool").expect_err("nothing for linux");
        assert!(matches!(err, BallError::NoMatchingAsset { .. }));
        let msg = format!("{}", err);
        assert!(msg.contains("tool-x86_64-apple-darwin.tar.gz"));
        assert!(msg.contains("tool-aarch64-apple-darwin.tar.gz"));
    }

    #[test]
    fn test_empty_release_errors_rather_than_guessing() {
        let err = select_asset(&[], &LINUX_X64, "tool").expect_err("no assets at all");
        assert!(format!("{}", err).contains("available: none"));
    }

    #[test]
    fn test_arm64_host_rejects_amd64_only_release() {
        let list = assets(&["tool-linux-amd64.tar.gz", "tool-linux-x86_64.tar.gz"]);
        assert!(select_asset(&list, &LINUX_ARM64, "tool").is_err());
    }

    #[test]
    fn test_amd64_host_rejects_32_bit_and_foreign_arches() {
        let list = assets(&[
            "tool-linux-i686.tar.gz",
            "tool-linux-armv7.tar.gz",
            "tool-linux-riscv64.tar.gz",
            "tool-linux-s390x.tar.gz",
        ]);
        assert!(select_asset(&list, &LINUX_X64, "tool").is_err());
    }

    #[test]
    fn test_matching_is_case_insensitive() {
        let list = assets(&["Tool-Linux-X86_64.ZIP"]);
        assert_eq!(pick(&list, &LINUX_X64), "Tool-Linux-X86_64.ZIP");
    }

    #[test]
    fn test_windows_host_skips_linux_builds() {
        let list = assets(&["tool-linux-amd64.tar.gz", "tool-win64.zip"]);
        assert_eq!(pick(&list, &WINDOWS_X64), "tool-win64.zip");
        assert_eq!(pick(&list, &LINUX_X64), "tool-linux-amd64.tar.gz");
    }

    #[test]
    fn test_host_platform_is_one_of_the_supported_pairs() {
        let spec = host_platform();
        assert!(matches!(spec.os, "linux" | "windows"));
        assert!(matches!(spec.arch, "x86_64" | "arm64"));
        assert!(!candidate_tags(&spec).is_empty());
    }
}
