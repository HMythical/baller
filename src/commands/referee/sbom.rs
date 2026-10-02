//! `baller referee sbom` — the roster as a CycloneDX 1.5 inventory.
//!
//! Built from what the roster recorded at install time: name, version,
//! source, `sha256`, `download_url`, repository and the dependency edges in
//! `package_dependencies`. Nothing is fetched, so it works with Referee
//! switched off.

use serde_json::{json, Value};

use super::export::{deliver, pretty};
use super::SbomFormat;
use crate::commands::draft::source_label;
use crate::context::AppContext;
use crate::core::db::InstalledPackage;
use crate::core::package::PackageSource;
use crate::error::error::BallError;

/// A roster row and the `(dep_name, dep_version)` edges recorded for it.
pub type SbomEntry = (InstalledPackage, Vec<(String, String)>);

pub fn execute_sbom(
    ctx: &AppContext,
    out: Option<&str>,
    format: SbomFormat,
) -> Result<(), BallError> {
    let mut entries: Vec<SbomEntry> = Vec::new();
    for row in ctx.db.list_packages()? {
        let deps = ctx.db.get_dependencies(&row.name)?;
        entries.push((row, deps));
    }

    let document = match format {
        SbomFormat::CyclonedxJson => cyclonedx(&entries),
    };
    deliver(&pretty(&document)?, out, "CycloneDX SBOM")
}

/// The CycloneDX 1.5 JSON document for a roster.
///
/// Every component is listed under `dependencies`, as the spec recommends, so
/// a package with no edges is distinguishable from one whose edges are
/// unknown. An edge to a package baller did not install (a system library, a
/// virtual package) has no component to point at and is left out.
pub fn cyclonedx(entries: &[SbomEntry]) -> Value {
    let components: Vec<Value> = entries.iter().map(|(row, _)| component(row)).collect();

    let dependencies: Vec<Value> = entries
        .iter()
        .map(|(row, deps)| {
            let depends_on: Vec<String> = deps
                .iter()
                .filter_map(|(dep_name, _)| {
                    entries
                        .iter()
                        .find(|(other, _)| &other.name == dep_name)
                        .map(|(other, _)| bom_ref(other))
                })
                .collect();
            json!({ "ref": bom_ref(row), "dependsOn": depends_on })
        })
        .collect();

    json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "tools": {
                "components": [{
                    "type": "application",
                    "name": "baller",
                    "version": env!("CARGO_PKG_VERSION"),
                }]
            }
        },
        "components": components,
        "dependencies": dependencies,
    })
}

fn bom_ref(row: &InstalledPackage) -> String {
    format!("{}@{}", row.name, row.version)
}

fn component(row: &InstalledPackage) -> Value {
    let pkg = row.to_package();

    let mut component = json!({
        "type": "application",
        "bom-ref": bom_ref(row),
        "name": row.name,
        "version": row.version,
        "properties": [
            { "name": "baller:source", "value": source_label(&pkg.source) },
            { "name": "baller:user_installed", "value": row.user_installed.to_string() },
        ],
    });

    if let Some(description) = row.description.as_deref().filter(|d| !d.is_empty()) {
        component["description"] = json!(description);
    }

    if let Some(purl) = purl(&pkg.source, &row.version) {
        component["purl"] = json!(purl);
    }

    // CycloneDX requires a SHA-256 to be 64 hex characters; anything else the
    // roster holds is not a hash a consumer can verify against.
    if let Some(sha) = row.sha256.as_deref().filter(|sha| is_sha256(sha)) {
        component["hashes"] = json!([{ "alg": "SHA-256", "content": sha.to_ascii_lowercase() }]);
    }

    let mut references = Vec::new();
    if let Some(url) = row.download_url.as_deref().filter(|u| !u.is_empty()) {
        references.push(json!({ "type": "distribution", "url": url }));
    }
    if let Some(url) = row.repository.as_deref().filter(|u| !u.is_empty()) {
        references.push(json!({ "type": "vcs", "url": url }));
    }
    if !references.is_empty() {
        component["externalReferences"] = json!(references);
    }

    component
}

/// A package URL, for the sources that have a registered purl type.
fn purl(source: &PackageSource, version: &str) -> Option<String> {
    match source {
        PackageSource::Cargo { crate_name } => {
            Some(format!("pkg:cargo/{}@{}", crate_name, version))
        }
        PackageSource::GitHub { owner, repo } => Some(format!(
            "pkg:github/{}/{}@{}",
            owner.to_ascii_lowercase(),
            repo.to_ascii_lowercase(),
            version
        )),
        _ => None,
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, source: &str, detail: &str) -> InstalledPackage {
        InstalledPackage {
            name: name.to_string(),
            version: "1.2.3".to_string(),
            source: source.to_string(),
            source_detail: Some(detail.to_string()),
            description: Some(format!("{} does things", name)),
            author: None,
            repository: Some(format!("https://github.com/owner/{}", name)),
            download_url: Some(format!("https://dl.test/{}.tar.gz", name)),
            sha256: Some("AB".repeat(32)),
            frozen: false,
            user_installed: true,
            install_path: String::new(),
            bin_path: None,
            manifest_path: None,
            installed_at: String::new(),
            advisory: None,
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn test_one_component_per_package_with_its_inventory() {
        let entries = vec![
            (row("tool", "github", "Owner/Tool"), Vec::new()),
            (row("ripgrep", "cargo", "ripgrep"), Vec::new()),
        ];
        let bom = cyclonedx(&entries);

        assert_eq!(bom["bomFormat"], json!("CycloneDX"));
        assert_eq!(bom["specVersion"], json!("1.5"));
        let components = bom["components"].as_array().unwrap();
        assert_eq!(components.len(), 2);

        let tool = &components[0];
        assert_eq!(tool["bom-ref"], json!("tool@1.2.3"));
        assert_eq!(tool["name"], json!("tool"));
        assert_eq!(tool["version"], json!("1.2.3"));
        assert_eq!(tool["purl"], json!("pkg:github/owner/tool@1.2.3"));
        assert_eq!(tool["hashes"][0]["alg"], json!("SHA-256"));
        assert_eq!(tool["hashes"][0]["content"], json!("ab".repeat(32)));
        assert_eq!(
            tool["externalReferences"][0],
            json!({ "type": "distribution", "url": "https://dl.test/tool.tar.gz" })
        );
        assert_eq!(tool["properties"][0]["value"], json!("github:Owner/Tool"));

        assert_eq!(components[1]["purl"], json!("pkg:cargo/ripgrep@1.2.3"));
    }

    #[test]
    fn test_dependencies_point_at_rostered_components_only() {
        let entries = vec![
            (
                row("app", "github", "owner/app"),
                vec![
                    ("lib".to_string(), ">=1.0".to_string()),
                    ("libc6".to_string(), "*".to_string()),
                ],
            ),
            (row("lib", "github", "owner/lib"), Vec::new()),
        ];
        let bom = cyclonedx(&entries);

        assert_eq!(
            bom["dependencies"],
            json!([
                { "ref": "app@1.2.3", "dependsOn": ["lib@1.2.3"] },
                { "ref": "lib@1.2.3", "dependsOn": [] },
            ])
        );
    }

    #[test]
    fn test_unverifiable_hash_and_empty_fields_are_left_out() {
        let mut bare = row("bare", "system", "apt");
        bare.sha256 = Some("not-a-hash".to_string());
        bare.download_url = None;
        bare.repository = None;
        bare.description = None;

        let component = component(&bare);
        assert!(component.get("hashes").is_none());
        assert!(component.get("externalReferences").is_none());
        assert!(component.get("description").is_none());
        assert!(component.get("purl").is_none());
    }

    #[test]
    fn test_an_empty_roster_is_a_valid_empty_bom() {
        let bom = cyclonedx(&[]);
        assert!(bom["components"].as_array().unwrap().is_empty());
        assert!(bom["dependencies"].as_array().unwrap().is_empty());
    }
}
