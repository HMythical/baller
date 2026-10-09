//! `--json` end to end: whatever a run does, stdout holds exactly one JSON
//! document (issue #16).
//!
//! These drive the real binary rather than calling commands in-process, since
//! the bugs they pin are about which stream a byte lands on: a prompt on
//! stdout, an error that never reaches it, clap printing help on its own.
//! Every case gets its own `--config` directory, so cases share nothing and
//! can run in parallel.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use serde_json::{json, Value};

/// The binary under test.
///
/// `BALLER_TEST_BIN` overrides cargo's path, so this test compiled for Windows
/// can be copied to a Windows host and pointed at the `baller.exe` beside it.
fn baller_bin() -> PathBuf {
    std::env::var_os("BALLER_TEST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_baller")))
}

/// A baller directory no other case can see.
struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join("baller_json_tests").join(format!(
            "{}_{}_{}",
            std::process::id(),
            tag,
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox { dir }
    }

    /// Run baller with `args` after `--config <sandbox>`.
    fn run(&self, args: &[&str]) -> Output {
        let mut full = vec!["--config", self.dir.to_str().unwrap()];
        full.extend_from_slice(args);
        self.run_raw(&full)
    }

    /// Run baller with exactly `args`, home directories pointed at the sandbox.
    fn run_raw(&self, args: &[&str]) -> Output {
        Command::new(baller_bin())
            .args(args)
            .env("HOME", &self.dir)
            .env("USERPROFILE", &self.dir)
            .env("LOCALAPPDATA", &self.dir)
            .env_remove("BALLER_LOG")
            .env_remove("CLICOLOR_FORCE")
            .stdin(Stdio::null())
            .output()
            .expect("run the baller binary")
    }

    fn write_conf(&self, contents: &str) {
        std::fs::write(self.dir.join("baller.conf"), contents).unwrap();
    }

    /// Put `name` on the roster as a cargo package, the way `draft` records one.
    fn seed_package(&self, name: &str, version: &str) {
        // The first run creates the database and its schema.
        assert!(self.run(&["--json", "roster"]).status.success());
        let db = rusqlite::Connection::open(self.dir.join("db").join("baller.db")).unwrap();
        db.execute(
            "INSERT INTO installed_packages (name, version, source, source_detail, install_path, bin_path)
             VALUES (?1, ?2, 'cargo', ?1, ?3, ?3)",
            rusqlite::params![
                name,
                version,
                self.dir.join("missing").to_string_lossy()
            ],
        )
        .unwrap();
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Stdout parsed as one JSON document; anything else on it fails the test.
fn document(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not a single JSON document ({}):\n--- stdout\n{}\n--- stderr\n{}",
            e,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

/// A failed run's JSON error: exit 1, the document on stdout, stderr silent.
fn error_document(out: &Output) -> Value {
    assert_eq!(out.status.code(), Some(1), "expected exit code 1");
    let value = document(out);
    assert!(
        out.stderr.is_empty(),
        "a --json error should leave stderr empty, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(value["error"].is_string(), "no error message in {}", value);
    value
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

// ---------------------------------------------------------------- help/version

#[test]
fn test_json_help_overview_is_a_document() {
    let sandbox = Sandbox::new("help_overview");
    let out = sandbox.run(&["--json", "help"]);
    assert!(out.status.success());
    let value = document(&out);
    assert_eq!(value["command"], "help");
    assert_eq!(value["commands"].as_array().unwrap().len(), 12);
    assert_eq!(value["injected"], json!([]));
    assert!(value["global_options"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["name"] == "--json"));
}

#[test]
fn test_json_help_topic_is_a_document() {
    let sandbox = Sandbox::new("help_topic");
    let value = document(&sandbox.run(&["--json", "help", "eject"]));
    assert_eq!(value["topic"], "eject");
    assert_eq!(value["kind"], "builtin");
    assert_eq!(value["usage"], "baller eject <PACKAGE_NAME>");
}

#[test]
fn test_json_help_and_version_flags_are_answered_in_json() {
    let sandbox = Sandbox::new("display_flags");

    for args in [
        vec!["--json", "--help"],
        vec!["--json", "-h"],
        vec!["--help", "--json"],
        // A top-level -h is answered before the subcommand after it, as clap does.
        vec!["--json", "--help", "draft"],
    ] {
        let out = sandbox.run(&args);
        assert!(out.status.success(), "{:?} failed", args);
        let value = document(&out);
        assert_eq!(value["command"], "help", "{:?}", args);
        assert!(value["topic"].is_null(), "{:?}", args);
    }

    for (args, topic) in [
        (vec!["--json", "draft", "--help"], "draft"),
        (vec!["draft", "-h", "--json"], "draft"),
        // draft's own --version takes a value; it is not a version request.
        (
            vec!["--json", "draft", "fd", "--version", "1.0.0", "--help"],
            "draft",
        ),
        (vec!["--json", "referee", "cache", "--help"], "referee"),
    ] {
        let value = document(&sandbox.run(&args));
        assert_eq!(value["topic"], topic, "{:?}", args);
    }

    for args in [
        vec!["--json", "-V"],
        vec!["--json", "--version"],
        vec!["--json", "version"],
    ] {
        let out = sandbox.run(&args);
        assert!(out.status.success(), "{:?} failed", args);
        assert_eq!(
            document(&out),
            json!({ "command": "version", "version": env!("CARGO_PKG_VERSION") }),
            "{:?}",
            args
        );
    }
}

#[test]
fn test_json_help_flag_keeps_the_config_directory() {
    let sandbox = Sandbox::new("help_config");
    let binary = baller_bin();
    std::fs::write(
        sandbox.dir.join("injected_commands.json"),
        json!([{
            "command_name": "my-tool",
            "description": "A helpful tool",
            "version": "1.0.0",
            "flags": [],
            "author": "",
            "require_root": false,
            "depends": [],
            "path": binary,
        }])
        .to_string(),
    )
    .unwrap();

    // --config sits before the help flag; the rewrite must carry it along, or
    // the injected commands stored under it go missing.
    let value = document(&sandbox.run(&["--json", "--help"]));
    assert_eq!(value["injected"][0]["name"], "my-tool");
}

#[test]
fn test_text_help_and_version_are_unchanged() {
    let sandbox = Sandbox::new("text_display");

    // clap names the binary as invoked: `baller.exe` on Windows.
    let out = sandbox.run(&["--help"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("[OPTIONS] <COMMAND>"));
    assert!(serde_json::from_slice::<Value>(&out.stdout).is_err());

    let out = sandbox.run(&["-V"]);
    assert_eq!(
        stdout(&out),
        format!("baller {}\n", env!("CARGO_PKG_VERSION"))
    );

    let out = sandbox.run(&["version"]);
    assert_eq!(
        stdout(&out),
        format!("Baller {}\n", env!("CARGO_PKG_VERSION"))
    );

    let out = sandbox.run(&["--no-color", "help"]);
    assert!(stdout(&out).starts_with("B.A.L.L.E.R - "));
    assert!(stdout(&out).contains("Commands:"));
}

// ---------------------------------------------------------------------- errors

#[test]
fn test_json_errors_are_documents_on_stdout() {
    let sandbox = Sandbox::new("errors");

    let value = error_document(&sandbox.run(&["--json", "help", "not-a-command"]));
    assert_eq!(value["command"], "help");
    assert_eq!(value["code"], "UnsupportedCommand");

    let missing = sandbox.dir.join("missing.toml");
    let value = error_document(&sandbox.run(&["--json", "build", missing.to_str().unwrap()]));
    assert_eq!(value["command"], "build");
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("manifest not found"));

    let value = error_document(&sandbox.run(&["--json", "eject", "not-installed"]));
    assert_eq!(value["command"], "eject");
    assert_eq!(value["code"], "PackageNotFound");
}

#[test]
fn test_json_cli_errors_are_documents_on_stdout() {
    let sandbox = Sandbox::new("cli_errors");

    // clap fails before it returns a parsed command; the subcommand is still
    // read off the raw arguments.
    let value = error_document(&sandbox.run(&["--json", "draft"]));
    assert_eq!(value["command"], "draft");
    assert_eq!(value["code"], "InvalidConfig");
    assert!(value["error"].as_str().unwrap().contains("<PACKAGE_NAME>"));

    let value = error_document(&sandbox.run(&["--json", "--bogus"]));
    assert!(value["command"].is_null());

    let value = error_document(&sandbox.run(&["--json"]));
    assert!(value["command"].is_null());
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("requires a subcommand"));
}

#[test]
fn test_text_errors_still_go_to_stderr() {
    let sandbox = Sandbox::new("text_errors");
    let out = sandbox.run(&["help", "not-a-command"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(stderr(&out).contains("[Error]: the following command does not exist"));
}

#[test]
fn test_json_after_an_injected_command_belongs_to_it() {
    let sandbox = Sandbox::new("external_json");
    // `--json` after an unknown subcommand is that command's argument, so
    // baller itself is not in JSON mode and reports the error as text.
    let out = sandbox.run(&["no-such-tool", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(stderr(&out).contains("no-such-tool"));
}

// --------------------------------------------------------------- confirmations

#[test]
fn test_json_eject_without_yes_asks_for_it_instead_of_prompting() {
    let sandbox = Sandbox::new("eject_confirm");
    sandbox.seed_package("some-pkg", "1.0.0");

    let value = error_document(&sandbox.run(&["--json", "eject", "some-pkg"]));
    assert_eq!(value["command"], "eject");
    assert_eq!(value["code"], "ConfirmationRequired");
    let message = value["error"].as_str().unwrap();
    assert!(message.contains("Are you sure you want to eject some-pkg?"));
    assert!(message.contains("--yes"));
    assert!(!message.contains("[y/N]"));

    // Nothing was ejected.
    let roster = document(&sandbox.run(&["--json", "roster"]));
    assert_eq!(roster["count"], 1);
}

#[test]
fn test_json_sweep_without_yes_asks_for_it_instead_of_prompting() {
    let sandbox = Sandbox::new("sweep_confirm");
    let cache = sandbox.dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let archive = cache.join("pkg-1.0.0.tar.gz");
    std::fs::write(&archive, vec![0u8; 2048]).unwrap();

    let value = error_document(&sandbox.run(&["--json", "sweep"]));
    assert_eq!(value["command"], "sweep");
    assert_eq!(value["code"], "ConfirmationRequired");
    assert!(archive.exists());

    let value = document(&sandbox.run(&["--json", "--yes", "sweep"]));
    assert_eq!(value["command"], "sweep");
    assert!(!archive.exists());
}

/// A `.ball` file for a command named `name` that runs baller itself.
fn ball_file(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(format!("{}.ball", name));
    std::fs::write(
        &path,
        format!(
            "[COMMAND-NAME]\nCOMMAND-NAME = \"{}\"\n\n[DESCRIPTION]\nDESCRIPTION = \"Test tool\"\n\n\
             [VERSION]\nVERSION = \"1.0.0\"\n\n[PATH]\nPATH = {}\n",
            name,
            baller_bin().display()
        ),
    )
    .unwrap();
    path
}

#[test]
fn test_json_inject_without_yes_asks_for_it_and_with_yes_reports_json() {
    let sandbox = Sandbox::new("inject");
    let ball = ball_file(&sandbox.dir, "json-tool");

    let value = error_document(&sandbox.run(&["--json", "inject", ball.to_str().unwrap()]));
    assert_eq!(value["command"], "inject");
    assert_eq!(value["code"], "ConfirmationRequired");

    let out = sandbox.run(&["--json", "--yes", "inject", ball.to_str().unwrap()]);
    assert!(out.status.success(), "inject failed: {}", stdout(&out));
    let value = document(&out);
    assert_eq!(value["command"], "inject");
    assert_eq!(value["name"], "json-tool");

    let help = document(&sandbox.run(&["--json", "help", "json-tool"]));
    assert_eq!(help["kind"], "injected");
    assert_eq!(help["description"], "Test tool");
}

// ------------------------------------------------------------ success commands

#[test]
fn test_json_read_only_commands_print_one_document() {
    let sandbox = Sandbox::new("read_only");
    sandbox.seed_package("serde", "1.0.229");

    for args in [
        vec!["--json", "roster"],
        vec!["--json", "roster", "serde"],
        vec!["--json", "freeze", "--list"],
        vec!["--json", "sweep", "--dry-run"],
        vec!["--json", "referee", "config"],
        vec!["--json", "referee", "cache"],
        vec!["--json", "referee", "sbom"],
        vec!["--json", "freeze", "serde"],
        vec!["--json", "freeze", "serde"],
    ] {
        let out = sandbox.run(&args);
        assert!(out.status.success(), "{:?} failed: {}", args, stderr(&out));
        document(&out);
    }
}

// --------------------------------------------------------- referee --fail-on

/// An OSV stand-in: every package matches one critical advisory.
fn critical_osv() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            answer(stream);
        }
    });
    base
}

fn answer(mut stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; length];
    let _ = reader.read_exact(&mut body);

    let response = if request_line.contains("/v1/vulns/") {
        json!({
            "id": "GHSA-test",
            "summary": "serde is affected",
            "aliases": ["CVE-GHSA-test"],
            "severity": [{
                "type": "CVSS_V3",
                "score": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"
            }],
            "affected": [{
                "package": { "ecosystem": "crates.io", "name": "serde" },
                "ranges": [{ "type": "SEMVER", "events": [{ "introduced": "0" }] }]
            }]
        })
    } else {
        let queries = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| v["queries"].as_array().map(Vec::len))
            .unwrap_or(1);
        let hit = json!({ "vulns": [{ "id": "GHSA-test", "modified": "2026-01-01T00:00:00Z" }] });
        json!({ "results": vec![hit; queries] })
    }
    .to_string();

    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.len(),
        response
    );
}

#[test]
fn test_fail_on_keeps_stdout_to_the_report() {
    let sandbox = Sandbox::new("fail_on");
    sandbox.write_conf(&format!("osv_base_url = {}\n", critical_osv()));
    sandbox.seed_package("serde", "1.0.229");

    for args in [
        vec![
            "--json",
            "referee",
            "audit",
            "--no-scan",
            "--fail-on",
            "block",
        ],
        vec!["--json", "referee", "check", "--fail-on", "block"],
    ] {
        let out = sandbox.run(&args);
        assert_eq!(out.status.code(), Some(1), "{:?} should fail", args);

        // One document, and it is the report: the failure is not a second one.
        let value = document(&out);
        assert_eq!(value["command"], "referee", "{:?}", args);
        assert!(value.get("error").is_none(), "{:?}", args);
        assert_eq!(value["packages"][0]["name"], "serde", "{:?}", args);

        assert!(
            stderr(&out).contains("[Error]"),
            "{:?}: the failure should be on stderr, got: {}",
            args,
            stderr(&out)
        );
    }
}
