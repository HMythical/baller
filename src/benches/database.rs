use baller::core::db::DbManager;
use baller::core::package::PackageSource;
use criterion::{black_box, Criterion};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub fn bench_db_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("database_operations");
    let counter = AtomicU64::new(0);

    group.bench_function("insert_and_retrieve", |b| {
        let i = counter.fetch_add(1, Ordering::SeqCst);
        let path = PathBuf::from(format!("/tmp/test_db_{}.db", i));

        let db = match DbManager::init_at_path(&path) {
            Ok(manager) => manager,
            Err(e) => panic!("Failed to init db: {}", e),
        };

        b.iter(|| {
            let name = format!("pkg_{}", i);
            let package = baller::core::package::Package {
                name: name.clone(),
                version: "1.0.0".to_string(),
                description: None,
                author: None,
                repository: None,
                architectures: None,
                dependencies: None,
                sha256: None,
                download_url: None,
                source: PackageSource::GitHub {
                    owner: "test".to_string(),
                    repo: "test".to_string(),
                },
            };

            let _ = db.insert_package(&package, "/install/path", None, None);
            let _ = black_box(db.get_package(&name));
        });

        let _ = std::fs::remove_file(&path);
    });

    group.finish();
}
