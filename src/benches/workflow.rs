use baller::core::db::DbManager;
use baller::core::dep_solver::{get_installed_map, resolve_deps};
use baller::core::package::PackageSource;
use baller::http::HttpClient;
use criterion::{black_box, Criterion};
use std::path::PathBuf;

pub fn bench_complete_workflow(c: &mut Criterion) {
    let mut group = c.benchmark_group("complete_workflow");

    group.bench_function("install_and_workflow", |b| {
        b.iter(|| {
            let db_path = PathBuf::from("/tmp/test_workflow.db");
            let db = match DbManager::init_at_path(&db_path) {
                Ok(manager) => manager,
                Err(e) => panic!("Failed to init db: {}", e),
            };

            let name = "workflow_pkg".to_string();
            let package = baller::core::package::Package {
                name: name.clone(),
                version: "1.0.0".to_string(),
                description: Some("Test package".to_string()),
                author: Some("Test Author".to_string()),
                repository: Some("https://github.com/test/repo".to_string()),
                architectures: None,
                dependencies: Some(vec![
                    "dep1 >=1.0".to_string(),
                    "? dep2".to_string(),
                    "dep3 >=2.0".to_string(),
                ]),
                sha256: Some("abc123def456".to_string()),
                hash_algorithm: None,
                download_url: Some("https://example.com/pkg.tar.gz".to_string()),
                source: PackageSource::GitHub {
                    owner: "owner".to_string(),
                    repo: name.clone(),
                },
            };

            let _ = get_installed_map(&db);
            let _ = resolve_deps(
                &name,
                &mut baller::core::registry::RegistryClient::new(HttpClient::new().unwrap()),
                &get_installed_map(&db),
            );

            db.insert_package(
                &package,
                "/install/path",
                Some("/bin/path"),
                Some("/manifest"),
                true,
            )
            .unwrap_or(());

            let _ = black_box(db.get_package(&name));
            let _ = black_box(db.get_dependencies(&name));
            let _ = black_box(db.remove_package(&name));

            let _ = std::fs::remove_file(&db_path);
        });
    });

    group.finish();
}
