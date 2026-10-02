//! Expanding a package into the identities advisory data actually knows it by.
//!
//! One `Package` is rarely one record in OSV. A Chocolatey package is a NuGet
//! id *and* usually a wrapper around an upstream GitHub project with its own
//! GHSA advisories; a crate is a crates.io package; an apt package is a Debian
//! one. Referee therefore asks about every identity it can justify and takes
//! the **worst** answer, because a vulnerability found under any identity is a
//! vulnerability in what gets installed.
//!
//! The rule that keeps this honest: an identity is only produced when the
//! package's own metadata supports it. Nothing is guessed from a name — a wrong
//! ecosystem would attach some other project's advisories to this install, and
//! that is worse than reporting `Unknown`.

use serde::Serialize;

use crate::core::manifest::parse_github_url;
use crate::core::package::{Package, PackageSource};

/// OSV ecosystem names, spelled the way the API expects them.
///
/// `GitHub` is repo-scoped (`owner/repo`) and is the only identity that works
/// the same on every OS, which makes it the spine of the gate. Its coverage in
/// public advisory data is thinner than the language ecosystems', so a `Clean`
/// verdict under it means "no record", and the gate reports it as exactly that.
pub const ECOSYSTEM_GITHUB: &str = "GitHub";
pub const ECOSYSTEM_CRATES: &str = "crates.io";
pub const ECOSYSTEM_NUGET: &str = "NuGet";
pub const ECOSYSTEM_DEBIAN: &str = "Debian";
pub const ECOSYSTEM_FEDORA: &str = "Fedora";

/// Why an identity exists.
///
/// This drives how much a verdict under it is trusted and how it is reported:
/// a `Fallback` hit is a best-effort mapping the user should be able to see as
/// such, while a `Primary` hit is the package's own ecosystem speaking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentityScope {
    /// The source's own ecosystem (Cargo → crates.io, GitHub → GitHub)
    Primary,
    /// Discovered from package metadata, such as a Chocolatey `project_url`
    Derived,
    /// Stated by the package itself, through a manifest `[advisory]` section
    Declared,
    /// A best-effort mapping that may be wrong (distro ecosystems)
    Fallback,
}

impl IdentityScope {
    pub fn label(self) -> &'static str {
        match self {
            IdentityScope::Primary => "primary",
            IdentityScope::Derived => "derived",
            IdentityScope::Declared => "declared",
            IdentityScope::Fallback => "fallback",
        }
    }

    /// How strongly this scope is believed, highest first. Used when the same
    /// `(ecosystem, name)` is produced twice by different routes.
    fn rank(self) -> u8 {
        match self {
            IdentityScope::Declared => 3,
            IdentityScope::Primary => 2,
            IdentityScope::Derived => 1,
            IdentityScope::Fallback => 0,
        }
    }
}

/// One resolvable identity against the OSV API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisoryIdentity {
    /// OSV ecosystem, e.g. `crates.io`, `NuGet`, `GitHub`, `Debian`
    pub ecosystem: String,
    /// Crate name, NuGet id, `owner/repo`, or distro source package
    pub name: String,
    pub scope: IdentityScope,
}

impl AdvisoryIdentity {
    pub fn new(ecosystem: &str, name: impl Into<String>, scope: IdentityScope) -> Self {
        Self {
            ecosystem: ecosystem.to_string(),
            name: name.into(),
            scope,
        }
    }

    /// `ecosystem:name`, as verdict lines and cache keys spell it.
    pub fn label(&self) -> String {
        format!("{}:{}", self.ecosystem, self.name)
    }
}

/// Every identity this package can be looked up under.
///
/// An empty result is a real answer: it means nothing advisory data understands
/// describes this package, and the caller reports `Unknown` rather than
/// inventing a mapping.
pub fn advisory_identities(pkg: &Package) -> Vec<AdvisoryIdentity> {
    let mut out = Vec::new();

    match &pkg.source {
        PackageSource::GitHub { owner, repo } => {
            if !owner.trim().is_empty() && !repo.trim().is_empty() {
                out.push(AdvisoryIdentity::new(
                    ECOSYSTEM_GITHUB,
                    format!("{}/{}", owner.trim(), repo.trim()),
                    IdentityScope::Primary,
                ));
            }
        }
        PackageSource::Cargo { crate_name } => {
            let name = if crate_name.trim().is_empty() {
                pkg.name.trim()
            } else {
                crate_name.trim()
            };
            if !name.is_empty() {
                out.push(AdvisoryIdentity::new(
                    ECOSYSTEM_CRATES,
                    name,
                    IdentityScope::Primary,
                ));
            }
        }
        PackageSource::Chocolatey { .. } => {
            if !pkg.name.trim().is_empty() {
                out.push(AdvisoryIdentity::new(
                    ECOSYSTEM_NUGET,
                    pkg.name.trim(),
                    IdentityScope::Primary,
                ));
            }
            // The Chocolatey path is the weakest advisory surface baller has:
            // NuGet records are filed against nuget.org ids, and a Chocolatey
            // wrapper for a tool like 7zip rarely has one. The upstream project
            // does, so the `project_url` baller already downloads is expanded
            // into a second identity and the worst of the two wins.
            if let Some((owner, repo)) = pkg.repository.as_deref().and_then(parse_github_url) {
                out.push(AdvisoryIdentity::new(
                    ECOSYSTEM_GITHUB,
                    format!("{}/{}", owner, repo),
                    IdentityScope::Derived,
                ));
            }
        }
        PackageSource::System { manager } => {
            // Distro version strings only line up with OSV heuristically, and
            // source-package names differ from binary ones, so these are
            // Fallback. pacman has no OSV ecosystem at all and produces
            // nothing rather than a guess.
            let ecosystem = match manager.trim().to_lowercase().as_str() {
                "apt" => Some(ECOSYSTEM_DEBIAN),
                "dnf" => Some(ECOSYSTEM_FEDORA),
                _ => None,
            };
            if let Some(ecosystem) = ecosystem {
                if !pkg.name.trim().is_empty() {
                    out.push(AdvisoryIdentity::new(
                        ecosystem,
                        pkg.name.trim(),
                        IdentityScope::Fallback,
                    ));
                }
            }
        }
        PackageSource::BallerRegistry { .. } => {
            // The registry serves no advisory data today (issue #10), and a
            // registry package's distribution shape maps to no OSV ecosystem.
            // Only what the package declares about itself is usable.
        }
    }

    // A self-declared identity applies to every source: it is the author
    // stating where their known-issue surface lives, which beats anything
    // inferred from the distribution channel.
    for declared in pkg.advisory_identities() {
        out.push(AdvisoryIdentity::new(
            &declared.0,
            declared.1,
            IdentityScope::Declared,
        ));
    }

    dedupe(out)
}

/// Collapse identities that name the same thing, keeping the strongest scope.
fn dedupe(identities: Vec<AdvisoryIdentity>) -> Vec<AdvisoryIdentity> {
    let mut out: Vec<AdvisoryIdentity> = Vec::with_capacity(identities.len());

    for identity in identities {
        if identity.ecosystem.trim().is_empty() || identity.name.trim().is_empty() {
            continue;
        }

        let existing = out.iter_mut().find(|kept| {
            kept.ecosystem.eq_ignore_ascii_case(&identity.ecosystem)
                && kept.name.eq_ignore_ascii_case(&identity.name)
        });

        match existing {
            Some(kept) => {
                if identity.scope.rank() > kept.scope.rank() {
                    kept.scope = identity.scope;
                }
            }
            None => out.push(identity),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::package::AdvisoryDeclaration;

    fn package(source: PackageSource) -> Package {
        Package {
            source,
            ..Package::new("tool", "1.0.0")
        }
    }

    #[test]
    fn test_github_source_maps_to_owner_repo() {
        let pkg = package(PackageSource::GitHub {
            owner: "BurntSushi".to_string(),
            repo: "ripgrep".to_string(),
        });
        let ids = advisory_identities(&pkg);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].ecosystem, ECOSYSTEM_GITHUB);
        assert_eq!(ids[0].name, "BurntSushi/ripgrep");
        assert_eq!(ids[0].scope, IdentityScope::Primary);
    }

    #[test]
    fn test_github_source_with_a_blank_half_produces_nothing() {
        let pkg = package(PackageSource::GitHub {
            owner: String::new(),
            repo: "ripgrep".to_string(),
        });
        assert!(advisory_identities(&pkg).is_empty());
    }

    #[test]
    fn test_cargo_source_maps_to_crates_io() {
        let pkg = package(PackageSource::Cargo {
            crate_name: "serde".to_string(),
        });
        let ids = advisory_identities(&pkg);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].ecosystem, ECOSYSTEM_CRATES);
        assert_eq!(ids[0].name, "serde");
    }

    #[test]
    fn test_cargo_source_falls_back_to_the_package_name() {
        let pkg = package(PackageSource::Cargo {
            crate_name: String::new(),
        });
        assert_eq!(advisory_identities(&pkg)[0].name, "tool");
    }

    #[test]
    fn test_chocolatey_derives_the_upstream_github_identity() {
        let mut pkg = package(PackageSource::Chocolatey {
            feed_url: "https://community.chocolatey.org/api/v2".to_string(),
        });
        pkg.name = "7zip".to_string();
        pkg.repository = Some("https://github.com/ip7z/7zip".to_string());

        let ids = advisory_identities(&pkg);
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].ecosystem, ECOSYSTEM_NUGET);
        assert_eq!(ids[0].name, "7zip");
        assert_eq!(ids[0].scope, IdentityScope::Primary);
        assert_eq!(ids[1].ecosystem, ECOSYSTEM_GITHUB);
        assert_eq!(ids[1].name, "ip7z/7zip");
        assert_eq!(ids[1].scope, IdentityScope::Derived);
    }

    #[test]
    fn test_chocolatey_with_a_non_github_project_url_stays_nuget_only() {
        let mut pkg = package(PackageSource::Chocolatey {
            feed_url: "feed".to_string(),
        });
        pkg.name = "7zip".to_string();
        pkg.repository = Some("https://www.7-zip.org/".to_string());

        let ids = advisory_identities(&pkg);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].ecosystem, ECOSYSTEM_NUGET);
    }

    #[test]
    fn test_chocolatey_with_no_project_url_stays_nuget_only() {
        let mut pkg = package(PackageSource::Chocolatey {
            feed_url: "feed".to_string(),
        });
        pkg.repository = None;
        assert_eq!(advisory_identities(&pkg).len(), 1);
    }

    #[test]
    fn test_apt_maps_to_debian_as_a_fallback() {
        let mut pkg = package(PackageSource::System {
            manager: "apt".to_string(),
        });
        pkg.name = "vim".to_string();
        let ids = advisory_identities(&pkg);
        assert_eq!(ids[0].ecosystem, ECOSYSTEM_DEBIAN);
        assert_eq!(ids[0].scope, IdentityScope::Fallback);
    }

    #[test]
    fn test_dnf_maps_to_fedora_as_a_fallback() {
        let pkg = package(PackageSource::System {
            manager: "DNF".to_string(),
        });
        assert_eq!(advisory_identities(&pkg)[0].ecosystem, ECOSYSTEM_FEDORA);
    }

    #[test]
    fn test_pacman_produces_no_identity_rather_than_a_guess() {
        let pkg = package(PackageSource::System {
            manager: "pacman".to_string(),
        });
        assert!(advisory_identities(&pkg).is_empty());
    }

    #[test]
    fn test_baller_registry_has_no_identity_today() {
        let pkg = package(PackageSource::BallerRegistry {
            url: "https://registry.baller.dev/api".to_string(),
        });
        assert!(advisory_identities(&pkg).is_empty());
    }

    #[test]
    fn test_declared_identity_covers_a_registry_package() {
        let mut pkg = package(PackageSource::BallerRegistry {
            url: "https://registry.baller.dev/api".to_string(),
        });
        pkg.advisory = Some(AdvisoryDeclaration {
            ecosystem: Some("crates.io".to_string()),
            name: Some("ripgrep".to_string()),
            aliases: Vec::new(),
        });

        let ids = advisory_identities(&pkg);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].ecosystem, "crates.io");
        assert_eq!(ids[0].name, "ripgrep");
        assert_eq!(ids[0].scope, IdentityScope::Declared);
    }

    #[test]
    fn test_declared_identity_defaults_its_name_to_the_package_name() {
        let mut pkg = package(PackageSource::BallerRegistry {
            url: "url".to_string(),
        });
        pkg.advisory = Some(AdvisoryDeclaration {
            ecosystem: Some("crates.io".to_string()),
            name: None,
            aliases: Vec::new(),
        });
        assert_eq!(advisory_identities(&pkg)[0].name, "tool");
    }

    #[test]
    fn test_declared_identity_without_an_ecosystem_is_ignored() {
        let mut pkg = package(PackageSource::BallerRegistry {
            url: "url".to_string(),
        });
        pkg.advisory = Some(AdvisoryDeclaration {
            ecosystem: None,
            name: Some("ripgrep".to_string()),
            aliases: Vec::new(),
        });
        assert!(advisory_identities(&pkg).is_empty());
    }

    #[test]
    fn test_a_declaration_upgrades_a_duplicate_primary_identity() {
        let mut pkg = package(PackageSource::Cargo {
            crate_name: "serde".to_string(),
        });
        pkg.advisory = Some(AdvisoryDeclaration {
            ecosystem: Some("crates.io".to_string()),
            name: Some("SERDE".to_string()),
            aliases: Vec::new(),
        });

        let ids = advisory_identities(&pkg);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].scope, IdentityScope::Declared);
    }

    #[test]
    fn test_identity_label_is_ecosystem_colon_name() {
        let id = AdvisoryIdentity::new(ECOSYSTEM_CRATES, "serde", IdentityScope::Primary);
        assert_eq!(id.label(), "crates.io:serde");
    }

    #[test]
    fn test_scope_labels() {
        assert_eq!(IdentityScope::Primary.label(), "primary");
        assert_eq!(IdentityScope::Derived.label(), "derived");
        assert_eq!(IdentityScope::Declared.label(), "declared");
        assert_eq!(IdentityScope::Fallback.label(), "fallback");
    }
}
