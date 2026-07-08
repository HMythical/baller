pub mod database;
pub mod dependency;
pub mod download;
pub mod workflow;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    benches,
    database::bench_db_operations,
    dependency::bench_dependency_operations,
    download::bench_download_performance,
    workflow::bench_complete_workflow,
);
criterion_main!(benches);
