use baller::core::db::DbManager;
use baller::core::dep_solver::{get_installed_map, resolve_deps};
use baller::core::package::PackageSource;
use baller::http::HttpClient;
use baller::security::scan::ArtifactScanner;
use baller::security::scoring::RefereeThresholds;
use baller::security::{FailPolicy, Referee};
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
                advisory: None,
                vulnerabilities: Vec::new(),
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

    // Referee's Phase A pass. The advisory service is never reached here: the
    // point is the cost baller adds around it — expanding identities, reading
    // the verdict cache, and banding the result — because that is what every
    // install pays whether or not an advisory exists.
    group.bench_function("referee_phase_a_cached", |b| {
        let db_path = PathBuf::from("/tmp/test_referee_bench.db");
        let _ = std::fs::remove_file(&db_path);
        let db = DbManager::init_at_path(&db_path).expect("init db");

        let plan: Vec<baller::core::package::Package> = (0..25)
            .map(|i| baller::core::package::Package {
                source: PackageSource::Cargo {
                    crate_name: format!("crate-{}", i),
                },
                ..baller::core::package::Package::new(&format!("crate-{}", i), "1.0.0")
            })
            .collect();

        // Pre-seed the cache so the benchmark measures the cached path, not
        // the network.
        for pkg in &plan {
            if let PackageSource::Cargo { crate_name } = &pkg.source {
                db.referee_cache_put("crates.io", crate_name, &pkg.version, "clean", None, "[]")
                    .expect("seed the referee cache");
            }
        }

        let referee = Referee::new(
            HttpClient::new().unwrap(),
            true,
            RefereeThresholds::default(),
            FailPolicy::FailOpen,
            // Unreachable on purpose: a cache hit must never dial out, and a
            // benchmark that silently started doing so would show it here.
            "http://127.0.0.1:1".to_string(),
            None,
            None,
        );

        b.iter(|| {
            let outcome = referee.gate(&db, black_box(&plan)).expect("gate the plan");
            black_box(outcome.reports.len());
        });

        let _ = std::fs::remove_file(&db_path);
    });

    // Phase B over a small extracted tree: the per-artifact cost of an install.
    group.bench_function("referee_phase_b_scan", |b| {
        let tree = std::env::temp_dir().join("baller_bench_scan_tree");
        let _ = std::fs::remove_dir_all(&tree);
        std::fs::create_dir_all(tree.join("bin")).expect("make the bench tree");
        std::fs::write(tree.join("README.md"), "# bench tool\n".repeat(200)).expect("write");
        std::fs::write(
            tree.join("bin/run.sh"),
            "#!/bin/sh\nexec \"$(dirname \"$0\")/tool\" \"$@\"\n",
        )
        .expect("write");
        std::fs::write(tree.join("bin/tool"), vec![0u8; 256 * 1024]).expect("write");

        let scanner = ArtifactScanner::new();
        b.iter(|| {
            black_box(scanner.scan_tree(black_box(&tree)).len());
        });

        let _ = std::fs::remove_dir_all(&tree);
    });

    group.finish();
}
