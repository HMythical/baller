use baller::core::dep_solver::resolve_deps;
use baller::core::registry::RegistryClient;
use baller::http::HttpClient;
use criterion::{black_box, Criterion};
use std::collections::HashMap;

pub fn bench_dependency_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("dependency_operations");

    group.bench_function("resolve_deps", |b| {
        let client = HttpClient::new().unwrap();
        let mut registry = RegistryClient::new(client);
        let installed_map = HashMap::new();

        b.iter(|| {
            let result = resolve_deps("test-pkg", &mut registry, &installed_map);
            let _ = black_box(result);
        });
    });

    group.finish();
}
