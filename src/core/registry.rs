use crate::core::package::{Package, PackageSource, Platform, SourceAffinity};
use crate::error::error::BallError;
use crate::http::cargo::CargoRegistry;
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
    Cargo,
}

impl RegistrySource {
    /// The name this source is written as in `baller.conf`'s `source_order`
    pub fn config_name(&self) -> &'static str {
        match self {
            RegistrySource::GitHub => "github",
            RegistrySource::BallerRegistry => "baller",
            RegistrySource::Chocolatey => "chocolatey",
            RegistrySource::System => "system",
            RegistrySource::Cargo => "cargo",
        }
    }

    /// The value stored in the database's `source` column for this source
    pub fn db_name(&self) -> &'static str {
        match self {
            RegistrySource::GitHub => "github",
            RegistrySource::BallerRegistry => "baller_registry",
            RegistrySource::Chocolatey => "chocolatey",
            RegistrySource::System => "system",
            RegistrySource::Cargo => "cargo",
        }
    }

    /// Parse a `source_order` entry; unknown names are ignored by the caller
    pub fn from_config_name(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "github" => Some(RegistrySource::GitHub),
            "baller" => Some(RegistrySource::BallerRegistry),
            "chocolatey" => Some(RegistrySource::Chocolatey),
            "system" => Some(RegistrySource::System),
            "cargo" => Some(RegistrySource::Cargo),
            _ => None,
        }
    }

    /// The registry a resolved package's source belongs to
    pub fn of(source: &PackageSource) -> Self {
        match source {
            PackageSource::GitHub { .. } => RegistrySource::GitHub,
            PackageSource::BallerRegistry { .. } => RegistrySource::BallerRegistry,
            PackageSource::Chocolatey { .. } => RegistrySource::Chocolatey,
            PackageSource::System { .. } => RegistrySource::System,
            PackageSource::Cargo { .. } => RegistrySource::Cargo,
        }
    }

    /// The platforms this source can serve, for checks made before a package
    /// exists (`--source`). Must agree with [`PackageSource::affinity`].
    pub fn affinity(&self) -> SourceAffinity {
        match self {
            RegistrySource::Chocolatey => SourceAffinity::Only(Platform::Windows),
            RegistrySource::System => SourceAffinity::Only(Platform::Linux),
            RegistrySource::GitHub | RegistrySource::BallerRegistry | RegistrySource::Cargo => {
                SourceAffinity::Agnostic
            }
        }
    }
}

/// Pre-flight: reject a source that cannot serve `host` before anything is
/// fetched from it. This is the `--source` half of [`ensure_installable_on`].
///
/// A mismatch is a hard error, never a fallback: the message names the
/// package, the source, the host and the fix.
pub fn ensure_source_supported(
    source: &RegistrySource,
    package: &str,
    host: Platform,
) -> Result<(), BallError> {
    ensure_affinity(package, source, source.affinity(), host)
}

fn ensure_affinity(
    package: &str,
    source: &RegistrySource,
    affinity: SourceAffinity,
    host: Platform,
) -> Result<(), BallError> {
    if affinity.supports(host) {
        return Ok(());
    }

    Err(BallError::UnsupportedOs(format!(
        "'{package}' uses the {} source, which only serves {} hosts, so it cannot be installed on {host} \
         — declare a [source.{host}] table for this platform in its manifest, or pass a different --source",
        source.config_name(),
        affinity.name(),
        host = host.name(),
    )))
}

/// Canonical spelling of a CPU architecture, so `amd64` and `x86_64` match.
pub fn normalize_arch(arch: &str) -> String {
    let lowered = arch.trim().to_lowercase();
    match lowered.as_str() {
        "amd64" | "x64" | "x86-64" => "x86_64".to_string(),
        "arm64" => "aarch64".to_string(),
        _ => lowered,
    }
}

/// A package's `architectures` is an allow-list; absent or empty means any.
fn ensure_arch_supported(pkg: &Package, host: Platform, host_arch: &str) -> Result<(), BallError> {
    let declared = match &pkg.architectures {
        Some(list) if !list.is_empty() => list,
        _ => return Ok(()),
    };

    let host_arch = normalize_arch(host_arch);
    if declared
        .iter()
        .any(|arch| normalize_arch(arch) == host_arch)
    {
        return Ok(());
    }

    Err(BallError::UnsupportedOs(format!(
        "'{}' only supports {} (its declared architectures), not this {}-{} host",
        pkg.name,
        declared.join(", "),
        host.name(),
        host_arch
    )))
}

/// Pre-flight: whether `pkg` can be installed on `host`/`host_arch` at all.
///
/// The single platform gate. It runs where a source is *used* — on the parsed
/// manifest, after `--source` is applied, and on every package the chain
/// resolved — so no path that builds a source differently can bypass it.
/// Checks the source's platform affinity, then the `architectures` allow-list.
pub fn ensure_installable_on(
    pkg: &Package,
    host: Platform,
    host_arch: &str,
) -> Result<(), BallError> {
    ensure_affinity(
        &pkg.name,
        &RegistrySource::of(&pkg.source),
        pkg.source.affinity(),
        host,
    )?;
    ensure_arch_supported(pkg, host, host_arch)
}

/// [`ensure_installable_on`] for the host this binary runs on
pub fn ensure_installable(pkg: &Package) -> Result<(), BallError> {
    ensure_installable_on(pkg, Platform::host(), std::env::consts::ARCH)
}

/// The source chain this platform prefers: the Baller registry first, then the
/// native ecosystem (Chocolatey on Windows, the distro package manager followed
/// by crates.io on Linux), with GitHub as the last-resort fallback.
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
            RegistrySource::Cargo,
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

pub trait RegistryIndex {
    fn fetch_package(&self, name: &str) -> Result<Package, BallError>;
}

impl RegistryIndex for RegistryClient {
    fn fetch_package(&self, name: &str) -> Result<Package, BallError> {
        self.fetch_package(name)
    }
}

pub struct RegistryClient {
    github: GitHubRegistry,
    baller_api: BallerRegistryApi,
    chocolatey: ChocolateyRegistry,
    system: SystemRegistry,
    cargo: CargoRegistry,
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
        let cargo = CargoRegistry::detect();

        Self {
            github,
            baller_api,
            chocolatey,
            system,
            cargo,
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
        let cargo = CargoRegistry::detect();

        Self {
            github,
            baller_api,
            chocolatey,
            system,
            cargo,
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

    /// `"cargo"` when a cargo toolchain was detected on this host
    pub fn cargo_manager_name(&self) -> Option<&'static str> {
        self.cargo.manager_name()
    }

    pub fn search(&self, query: &str) -> Result<Vec<Package>, BallError> {
        let mut all_results = Vec::new();

        for source in &self.source_order {
            let results = match source {
                RegistrySource::GitHub => self.github.search(query),
                RegistrySource::BallerRegistry => self.baller_api.search(query),
                RegistrySource::Chocolatey => self.chocolatey.search(query),
                RegistrySource::System => self.system.search(query),
                RegistrySource::Cargo => self.cargo.search(query),
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
            RegistrySource::Cargo => self.cargo.fetch_package(name),
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
            RegistrySource::Cargo => Err(BallError::PackageManagerError(format!(
                "cargo packages always install the latest available version — cannot pin '{}'",
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

    const ALL_SOURCES: [RegistrySource; 5] = [
        RegistrySource::GitHub,
        RegistrySource::BallerRegistry,
        RegistrySource::Chocolatey,
        RegistrySource::System,
        RegistrySource::Cargo,
    ];

    fn package_from(source: &RegistrySource) -> Package {
        let mut pkg = Package::new("tool", "1.0.0");
        pkg.source = match source {
            RegistrySource::GitHub => PackageSource::GitHub {
                owner: "o".to_string(),
                repo: "tool".to_string(),
            },
            RegistrySource::BallerRegistry => PackageSource::BallerRegistry {
                url: "https://reg.example.com".to_string(),
            },
            RegistrySource::Chocolatey => PackageSource::Chocolatey {
                feed_url: "https://feed.example.com".to_string(),
            },
            RegistrySource::System => PackageSource::System {
                manager: "apt".to_string(),
            },
            RegistrySource::Cargo => PackageSource::Cargo {
                crate_name: "tool".to_string(),
            },
        };
        pkg
    }

    #[test]
    fn test_registry_and_package_affinity_agree() {
        for source in &ALL_SOURCES {
            let pkg = package_from(source);
            assert_eq!(&RegistrySource::of(&pkg.source), source);
            assert_eq!(source.affinity(), pkg.source.affinity(), "{:?}", source);
        }
    }

    #[test]
    fn test_affinity_gate_every_source_on_both_hosts() {
        let expected = [
            (RegistrySource::GitHub, true, true),
            (RegistrySource::BallerRegistry, true, true),
            (RegistrySource::Chocolatey, false, true),
            (RegistrySource::System, true, false),
            (RegistrySource::Cargo, true, true),
        ];

        for (source, on_linux, on_windows) in expected {
            let pkg = package_from(&source);
            for (host, allowed) in [(Platform::Linux, on_linux), (Platform::Windows, on_windows)] {
                let result = ensure_installable_on(&pkg, host, "x86_64");
                assert_eq!(result.is_ok(), allowed, "{:?} on {:?}", source, host);
                if let Err(err) = result {
                    assert!(matches!(err, BallError::UnsupportedOs(_)));
                }
            }
        }
    }

    #[test]
    fn test_affinity_mismatch_names_package_source_host_and_fix() {
        let pkg = package_from(&RegistrySource::Chocolatey);
        let msg = ensure_installable_on(&pkg, Platform::Linux, "x86_64")
            .unwrap_err()
            .to_string();
        assert!(msg.contains("'tool'"), "{}", msg);
        assert!(msg.contains("chocolatey source"), "{}", msg);
        assert!(msg.contains("only serves windows hosts"), "{}", msg);
        assert!(msg.contains("cannot be installed on linux"), "{}", msg);
        assert!(msg.contains("[source.linux]"), "{}", msg);
        assert!(msg.contains("--source"), "{}", msg);

        let system = package_from(&RegistrySource::System);
        let msg = ensure_installable_on(&system, Platform::Windows, "x86_64")
            .unwrap_err()
            .to_string();
        assert!(msg.contains("[source.windows]"), "{}", msg);
    }

    #[test]
    fn test_source_flag_gate_runs_without_a_package() {
        assert!(
            ensure_source_supported(&RegistrySource::Chocolatey, "7zip", Platform::Linux).is_err()
        );
        assert!(
            ensure_source_supported(&RegistrySource::System, "vim", Platform::Windows).is_err()
        );
        assert!(
            ensure_source_supported(&RegistrySource::Chocolatey, "7zip", Platform::Windows).is_ok()
        );
        assert!(ensure_source_supported(&RegistrySource::System, "vim", Platform::Linux).is_ok());
    }

    #[test]
    fn test_normalize_arch_aliases() {
        assert_eq!(normalize_arch("amd64"), "x86_64");
        assert_eq!(normalize_arch("X64"), "x86_64");
        assert_eq!(normalize_arch("x86-64"), "x86_64");
        assert_eq!(normalize_arch("x86_64"), "x86_64");
        assert_eq!(normalize_arch("arm64"), "aarch64");
        assert_eq!(normalize_arch(" AArch64 "), "aarch64");
        assert_eq!(normalize_arch("riscv64"), "riscv64");
    }

    #[test]
    fn test_architectures_is_an_enforced_allow_list() {
        let mut pkg = package_from(&RegistrySource::GitHub);

        pkg.architectures = None;
        assert!(ensure_installable_on(&pkg, Platform::Linux, "aarch64").is_ok());

        pkg.architectures = Some(Vec::new());
        assert!(ensure_installable_on(&pkg, Platform::Linux, "aarch64").is_ok());

        pkg.architectures = Some(vec!["x86_64".to_string()]);
        assert!(ensure_installable_on(&pkg, Platform::Linux, "x86_64").is_ok());
        let err = ensure_installable_on(&pkg, Platform::Linux, "aarch64").unwrap_err();
        assert!(matches!(err, BallError::UnsupportedOs(_)));
        let msg = err.to_string();
        assert!(msg.contains("'tool' only supports x86_64"), "{}", msg);
        assert!(msg.contains("linux-aarch64"), "{}", msg);

        // Aliases on either side compare equal
        pkg.architectures = Some(vec!["amd64".to_string(), "arm64".to_string()]);
        assert!(ensure_installable_on(&pkg, Platform::Windows, "x86_64").is_ok());
        assert!(ensure_installable_on(&pkg, Platform::Linux, "aarch64").is_ok());
    }

    #[test]
    fn test_affinity_is_checked_before_architectures() {
        let mut pkg = package_from(&RegistrySource::Chocolatey);
        pkg.architectures = Some(vec!["aarch64".to_string()]);
        let msg = ensure_installable_on(&pkg, Platform::Linux, "x86_64")
            .unwrap_err()
            .to_string();
        assert!(msg.contains("chocolatey source"), "{}", msg);
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
            assert!(!order.contains(&RegistrySource::Cargo));
        } else {
            assert_eq!(
                order,
                vec![
                    RegistrySource::BallerRegistry,
                    RegistrySource::System,
                    RegistrySource::Cargo,
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
            RegistrySource::Cargo,
        ] {
            let name = source.config_name();
            assert_eq!(RegistrySource::from_config_name(name), Some(source));
        }
    }

    #[test]
    fn test_cargo_source_names() {
        assert_eq!(RegistrySource::Cargo.config_name(), "cargo");
        assert_eq!(RegistrySource::Cargo.db_name(), "cargo");
        assert_eq!(
            RegistrySource::from_config_name(" Cargo "),
            Some(RegistrySource::Cargo)
        );
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
