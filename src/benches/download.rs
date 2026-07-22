use baller::core::downloader::Downloader;
use baller::core::package::PackageSource;
use baller::http::HttpClient;
use criterion::{black_box, Criterion};
use std::path::PathBuf;

pub fn bench_download_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("download_performance");

    group.bench_function("download_and_extract", |b| {
        let downloader = Downloader::new(
            PathBuf::from("/tmp/test_download_cache"),
            HttpClient::new().unwrap(),
        );

        let pkg = baller::core::package::Package {
            name: "test-pkg".to_string(),
            version: "1.0.0".to_string(),
            description: None,
            author: None,
            repository: None,
            architectures: None,
            dependencies: None,
            sha256: None,
            hash_algorithm: None,
            download_url: None,
            source: PackageSource::GitHub {
                owner: "test".to_string(),
                repo: "test".to_string(),
            },
        };

        b.iter(|| {
            let _ = black_box(downloader.download_and_extract(&pkg, false));
        });
    });

    group.finish();
}
