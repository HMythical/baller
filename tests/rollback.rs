//! A failed `draft`, `substitute` or `update` leaves nothing of itself behind
//! (#59).
//!
//! Each case drives the real binary against a mock baller registry, makes one
//! step partway through the plan fail — a 404 archive or a vetoing pre-hook —
//! and then checks the roster, the links and the cache directly. Packages the
//! run installed fresh must be gone; packages it found already installed must
//! be back at the version, link and roster row they had.
//!
//! Which version a package is exposed as is read from the binary itself — each
//! archive holds a script that names its version — so the same checks cover a
//! Unix symlink under `$HOME/.local/bin` and the copy Windows makes under
//! `%LOCALAPPDATA%\baller\bin`. `BALLER_TEST_BIN` overrides the binary, as in
//! `json_output.rs`, so a Windows build of this test can run on a Windows host.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};

/// The binary under test.
fn baller_bin() -> PathBuf {
    std::env::var_os("BALLER_TEST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_baller")))
}

/// A baller registry stand-in whose packages and archives can change between
/// runs, the way a new release appears upstream.
#[derive(Clone)]
struct Registry {
    base: String,
    packages: Arc<Mutex<HashMap<String, Value>>>,
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl Registry {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let registry = Registry {
            base: format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port()),
            packages: Arc::default(),
            files: Arc::default(),
        };
        let serving = registry.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serving.answer(stream);
            }
        });
        registry
    }

    /// Publish `name` at `version`. Its archive is served unless `broken`.
    fn publish(&self, name: &str, version: &str, deps: &[&str], broken: bool) {
        let file = format!("{}-{}.tar.gz", name, version);
        self.packages.lock().unwrap().insert(
            name.to_string(),
            json!({
                "name": name,
                "version": version,
                "dependencies": deps,
                "download_url": format!("{}/files/{}", self.base, file),
            }),
        );
        let mut files = self.files.lock().unwrap();
        if broken {
            files.remove(&file);
        } else {
            files.insert(file, archive(name, version));
        }
    }

    fn answer(&self, mut stream: TcpStream) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            return;
        }
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
        }
        let path = request_line.split_whitespace().nth(1).unwrap_or("");

        let found = if let Some(name) = path.strip_prefix("/api/packages/") {
            self.packages
                .lock()
                .unwrap()
                .get(name)
                .map(|pkg| pkg.to_string().into_bytes())
        } else if let Some(file) = path.strip_prefix("/files/") {
            self.files.lock().unwrap().get(file).cloned()
        } else {
            None
        };

        let (status, body) = match found {
            Some(body) => ("200 OK", body),
            None => ("404 Not Found", b"{}".to_vec()),
        };
        let _ = write!(
            stream,
            "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            status,
            body.len()
        );
        let _ = stream.write_all(&body);
    }
}

/// A `.tar.gz` holding one runnable script named after the package.
fn archive(name: &str, version: &str) -> Vec<u8> {
    let script = format!("#!/bin/sh\necho {} {}\n", name, version);
    let mut header = tar::Header::new_gnu();
    header.set_size(script.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();

    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    builder
        .append_data(&mut header, name, script.as_bytes())
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

/// A baller directory and `HOME` no other case can see.
struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str, registry: &Registry) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir()
            .join("baller_rollback_tests")
            .join(format!(
                "{}_{}_{}",
                std::process::id(),
                tag,
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("baller.conf"),
            format!(
                "source_order = baller\nballer_registry_url = {}/api\n",
                registry.base
            ),
        )
        .unwrap();
        Sandbox { dir }
    }

    /// Run baller with `args`, Referee off: these cases are about rollback,
    /// not about what the gate would say.
    fn run(&self, args: &[&str]) -> Output {
        let mut full = vec!["--config", self.dir.to_str().unwrap(), "--no-referee"];
        full.extend_from_slice(args);
        Command::new(baller_bin())
            .args(&full)
            .env("HOME", &self.dir)
            .env("USERPROFILE", &self.dir)
            .env("LOCALAPPDATA", &self.dir)
            .env_remove("BALLER_LOG")
            .stdin(Stdio::null())
            .output()
            .expect("run the baller binary")
    }

    fn succeeds(&self, args: &[&str]) {
        let out = self.run(args);
        assert!(out.status.success(), "{:?} failed: {}", args, stderr(&out));
    }

    /// Run `args`, which must fail, and return its stderr.
    fn fails(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(!out.status.success(), "{:?} should have failed", args);
        stderr(&out)
    }

    /// A pre-hook for `package` that vetoes the operation.
    fn veto(&self, package: &str, hook: &str) {
        let hooks = self.dir.join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        #[cfg(windows)]
        let (file, body) = (
            format!("{}_{}.ps1", package, hook),
            "Write-Output vetoed\nexit 1\n",
        );
        #[cfg(not(windows))]
        let (file, body) = (format!("{}_{}.sh", package, hook), "echo vetoed\nexit 1\n");
        std::fs::write(hooks.join(file), body).unwrap();
    }

    /// The roster row for `name`: `(version, user_installed)`.
    fn row(&self, name: &str) -> Option<(String, bool)> {
        let db = rusqlite::Connection::open(self.dir.join("db").join("baller.db")).unwrap();
        db.query_row(
            "SELECT version, user_installed FROM installed_packages WHERE name = ?1",
            [name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()
    }

    fn roster(&self) -> Vec<String> {
        let db = rusqlite::Connection::open(self.dir.join("db").join("baller.db")).unwrap();
        let mut stmt = db
            .prepare("SELECT name FROM installed_packages ORDER BY name")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// The file baller exposes `name` as: a symlink on Unix, a copy on Windows.
    fn exposed(&self, name: &str) -> PathBuf {
        #[cfg(windows)]
        let path = self
            .dir
            .join("baller")
            .join("bin")
            .join(format!("{}.exe", name));
        #[cfg(not(windows))]
        let path = self.dir.join(".local").join("bin").join(name);
        path
    }

    /// The version `name`'s exposed binary runs, read from the script in it;
    /// `None` when nothing is exposed or the symlink dangles.
    fn exposed_version(&self, name: &str) -> Option<String> {
        let script = std::fs::read_to_string(self.exposed(name)).ok()?;
        script
            .lines()
            .nth(1)?
            .strip_prefix(&format!("echo {} ", name))
            .map(str::to_string)
    }

    fn extract_dir(&self, name: &str, version: &str) -> PathBuf {
        self.dir.join("cache").join(format!("{}-{}", name, version))
    }

    /// Whether any cached archive of `name` at `version` is left.
    fn archive_cached(&self, name: &str, version: &str) -> bool {
        let suffix = format!("{}-{}.tar.gz", name, version);
        std::fs::read_dir(self.dir.join("cache"))
            .unwrap()
            .flatten()
            .any(|entry| {
                entry.path().is_file() && entry.file_name().to_string_lossy().ends_with(&suffix)
            })
    }

    /// Assert `name` was installed by the failed run and is now fully gone.
    fn assert_gone(&self, name: &str, version: &str) {
        assert_eq!(self.row(name), None, "{} is still on the roster", name);
        assert!(
            std::fs::symlink_metadata(self.exposed(name)).is_err(),
            "{} is still exposed at {}",
            name,
            self.exposed(name).display()
        );
        assert!(
            !self.extract_dir(name, version).exists(),
            "{} v{} is still extracted",
            name,
            version
        );
        assert!(
            !self.archive_cached(name, version),
            "{} v{}'s archive is still cached",
            name,
            version
        );
    }

    /// Assert `name` is installed at `version`, linked to that version's binary.
    fn assert_installed(&self, name: &str, version: &str) {
        let (installed, _) = self
            .row(name)
            .unwrap_or_else(|| panic!("{} is not on the roster", name));
        assert_eq!(installed, version, "{} is at the wrong version", name);
        assert_eq!(
            self.exposed_version(name).as_deref(),
            Some(version),
            "{} is exposed at the wrong version",
            name
        );
        let target = self.extract_dir(name, version).join(name);
        #[cfg(not(windows))]
        assert_eq!(
            std::fs::read_link(self.exposed(name)).ok(),
            Some(target.clone()),
            "{} link",
            name
        );
        assert!(
            target.exists(),
            "{}'s binary {} is gone",
            name,
            target.display()
        );
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// Assert the rollback warning names exactly `names`, in any order: the
/// resolver may install independent dependencies either way round.
fn assert_reports_rollback(stderr: &str, names: &[&str]) {
    let marker = "undid this run's changes to ";
    let mut reported: Vec<&str> = stderr
        .lines()
        .find_map(|line| line.split_once(marker).map(|(_, rest)| rest))
        .unwrap_or_else(|| panic!("no rollback report, got:\n{}", stderr))
        .split(", ")
        .collect();
    reported.sort_unstable();
    let mut expected = names.to_vec();
    expected.sort_unstable();
    assert_eq!(reported, expected, "stderr:\n{}", stderr);
}

#[test]
fn test_a_failed_draft_removes_the_dependencies_it_installed() {
    let registry = Registry::start();
    registry.publish("liba", "1.0.0", &[], false);
    registry.publish("libb", "1.0.0", &[], false);
    registry.publish("app", "1.0.0", &["liba", "libb"], true);
    let sandbox = Sandbox::new("draft_fresh", &registry);

    let err = sandbox.fails(&["draft", "app"]);

    assert_reports_rollback(&err, &["liba", "libb"]);
    sandbox.assert_gone("liba", "1.0.0");
    sandbox.assert_gone("libb", "1.0.0");
    assert!(sandbox.roster().is_empty(), "{:?}", sandbox.roster());
}

#[test]
fn test_a_vetoed_root_rolls_back_its_dependencies() {
    let registry = Registry::start();
    registry.publish("liba", "1.0.0", &[], false);
    registry.publish("app", "1.0.0", &["liba"], false);
    let sandbox = Sandbox::new("draft_veto", &registry);
    sandbox.veto("app", "pre_install");

    let err = sandbox.fails(&["draft", "app"]);

    assert_reports_rollback(&err, &["liba"]);
    sandbox.assert_gone("liba", "1.0.0");
    sandbox.assert_gone("app", "1.0.0");
}

#[test]
fn test_a_failed_forced_draft_restores_what_it_reinstalled() {
    let registry = Registry::start();
    registry.publish("liba", "1.0.0", &[], false);
    let sandbox = Sandbox::new("draft_force", &registry);
    sandbox.succeeds(&["draft", "liba"]);

    // A new liba appears, and an app that needs it but cannot be fetched.
    registry.publish("liba", "2.0.0", &[], false);
    registry.publish("app", "1.0.0", &["liba"], true);
    let err = sandbox.fails(&["draft", "app", "--force"]);

    assert_reports_rollback(&err, &["liba"]);
    // liba was reinstalled at 2.0.0 as a dependency; it comes back as the
    // 1.0.0 the user drafted, still marked as user-installed.
    sandbox.assert_installed("liba", "1.0.0");
    assert_eq!(sandbox.row("liba"), Some(("1.0.0".to_string(), true)));
    assert!(!sandbox.extract_dir("liba", "2.0.0").exists());
    assert_eq!(sandbox.roster(), vec!["liba"]);
}

#[test]
fn test_a_failed_substitute_leaves_both_packages_as_they_were() {
    let registry = Registry::start();
    registry.publish("old", "1.0.0", &[], false);
    registry.publish("shared", "1.0.0", &[], false);
    let sandbox = Sandbox::new("substitute", &registry);
    sandbox.succeeds(&["draft", "old"]);
    sandbox.succeeds(&["draft", "shared"]);

    // The replacement needs a newer `shared` and then cannot be fetched.
    registry.publish("shared", "2.0.0", &[], false);
    registry.publish("new", "1.0.0", &["shared"], true);
    let err = sandbox.fails(&["--yes", "substitute", "old", "new"]);

    assert_reports_rollback(&err, &["shared"]);
    sandbox.assert_installed("old", "1.0.0");
    sandbox.assert_installed("shared", "1.0.0");
    assert!(!sandbox.extract_dir("shared", "2.0.0").exists());
    assert_eq!(sandbox.row("new"), None);
    assert_eq!(sandbox.roster(), vec!["old", "shared"]);
}

#[test]
fn test_a_failed_update_keeps_the_old_release_and_drops_new_dependencies() {
    let registry = Registry::start();
    registry.publish("tool", "1.0.0", &[], false);
    let sandbox = Sandbox::new("update_fetch", &registry);
    sandbox.succeeds(&["draft", "tool"]);

    // 2.0.0 pulls in a new dependency, then its own archive is missing.
    registry.publish("libnew", "1.0.0", &[], false);
    registry.publish("tool", "2.0.0", &["libnew"], true);
    let err = sandbox.fails(&["update", "tool"]);

    assert_reports_rollback(&err, &["libnew"]);
    sandbox.assert_gone("libnew", "1.0.0");
    // The old extract dir is pruned only after a committed update.
    sandbox.assert_installed("tool", "1.0.0");
    assert_eq!(sandbox.roster(), vec!["tool"]);
}

#[test]
fn test_a_vetoed_update_drops_the_dependencies_it_fetched() {
    let registry = Registry::start();
    registry.publish("tool", "1.0.0", &[], false);
    let sandbox = Sandbox::new("update_veto", &registry);
    sandbox.succeeds(&["draft", "tool"]);

    registry.publish("libnew", "1.0.0", &[], false);
    registry.publish("tool", "2.0.0", &["libnew"], false);
    sandbox.veto("tool", "pre_update");
    let err = sandbox.fails(&["update", "tool"]);

    assert_reports_rollback(&err, &["libnew"]);
    sandbox.assert_gone("libnew", "1.0.0");
    sandbox.assert_installed("tool", "1.0.0");
    assert!(!sandbox.extract_dir("tool", "2.0.0").exists());
}

#[test]
fn test_a_successful_update_still_prunes_the_old_release() {
    let registry = Registry::start();
    registry.publish("tool", "1.0.0", &[], false);
    let sandbox = Sandbox::new("update_ok", &registry);
    sandbox.succeeds(&["draft", "tool"]);

    registry.publish("libnew", "1.0.0", &[], false);
    registry.publish("tool", "2.0.0", &["libnew"], false);
    sandbox.succeeds(&["update", "tool"]);

    sandbox.assert_installed("tool", "2.0.0");
    sandbox.assert_installed("libnew", "1.0.0");
    assert!(!sandbox.extract_dir("tool", "1.0.0").exists());
}

#[test]
fn test_a_successful_draft_reports_what_it_installed() {
    let registry = Registry::start();
    registry.publish("liba", "1.0.0", &[], false);
    registry.publish("app", "1.0.0", &["liba"], false);
    let sandbox = Sandbox::new("draft_ok", &registry);

    let out = sandbox.run(&["--json", "draft", "app"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["installed"], json!(["liba", "app"]));
    sandbox.assert_installed("liba", "1.0.0");
    sandbox.assert_installed("app", "1.0.0");
}
