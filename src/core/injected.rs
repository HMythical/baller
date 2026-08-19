use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::config::BallerConfig;
use crate::core::ball_parser::BallManifest;
use crate::error::error::BallError;
use crate::utils::fs::atomic_write;

/// File under baller's config directory holding every injected command.
pub const INJECTED_FILE: &str = "injected_commands.json";

/// An injected command as persisted in `injected_commands.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InjectedCommand {
    pub command_name: String,
    pub description: String,
    pub version: String,
    pub flags: Vec<String>,
    pub author: String,
    pub require_root: bool,
    pub depends: Vec<String>,
    pub path: PathBuf,
}

impl From<BallManifest> for InjectedCommand {
    fn from(manifest: BallManifest) -> Self {
        Self {
            command_name: manifest.command_name,
            description: manifest.description,
            version: manifest.version,
            flags: manifest.flags,
            author: manifest.author,
            require_root: manifest.require_root,
            depends: manifest.depends,
            path: manifest.path,
        }
    }
}

/// Path of the injected commands store inside `baller_dir`.
pub fn injected_path(baller_dir: &str) -> PathBuf {
    Path::new(baller_dir).join(INJECTED_FILE)
}

/// Baller's config directory, derived from an already loaded config.
///
/// Every baller-managed path hangs off that directory, and `hooks_dir` is not
/// user-overridable, so its parent is authoritative.
pub fn resolve_baller_dir(config: &BallerConfig) -> String {
    config
        .hooks_dir
        .parent()
        .unwrap_or(config.hooks_dir.as_path())
        .to_string_lossy()
        .to_string()
}

/// Loads every injected command. A missing store means "nothing injected yet";
/// an unreadable one is reported but never blocks baller's built-in commands.
pub fn load_injected(baller_dir: &str) -> Vec<InjectedCommand> {
    let path = injected_path(baller_dir);

    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };

    match serde_json::from_str(&content) {
        Ok(commands) => commands,
        Err(e) => {
            eprintln!("[Warning]: ignoring malformed '{}': {}", path.display(), e);
            Vec::new()
        }
    }
}

/// Writes the injected command store, replacing its previous contents.
pub fn save_injected(baller_dir: &str, commands: &[InjectedCommand]) -> Result<(), BallError> {
    let content = serde_json::to_vec_pretty(commands).map_err(|e| {
        BallError::InjectedCommandError(format!("failed to serialize injected commands: {}", e))
    })?;

    atomic_write(&injected_path(baller_dir), &content)
}

/// Finds an injected command by the name users type after `baller`.
pub fn find_command(baller_dir: &str, name: &str) -> Option<InjectedCommand> {
    load_injected(baller_dir)
        .into_iter()
        .find(|c| c.command_name == name)
}

/// Adds a command to the store, replacing any existing entry with the same name.
pub fn upsert_injected(baller_dir: &str, command: InjectedCommand) -> Result<(), BallError> {
    let mut commands = load_injected(baller_dir);
    commands.retain(|c| c.command_name != command.command_name);
    commands.push(command);
    save_injected(baller_dir, &commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_dir() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join("baller_injected_tests")
            .join(format!("{}_{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    fn sample(name: &str) -> InjectedCommand {
        InjectedCommand {
            command_name: name.to_string(),
            description: "does things".to_string(),
            version: "1.0.0".to_string(),
            flags: vec!["-y".to_string(), "--yes".to_string()],
            author: "HMythical".to_string(),
            require_root: false,
            depends: vec!["python3".to_string()],
            path: PathBuf::from("/usr/local/bin/my-tool"),
        }
    }

    #[test]
    fn test_injected_path_is_under_baller_dir() {
        let path = injected_path("/home/user/.baller");
        assert_eq!(
            path,
            PathBuf::from("/home/user/.baller").join(INJECTED_FILE)
        );
    }

    #[test]
    fn test_load_missing_store_is_empty() {
        let dir = test_dir();
        assert!(load_injected(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = test_dir();
        let commands = vec![sample("my-tool"), sample("other-tool")];
        save_injected(&dir, &commands).unwrap();

        let loaded = load_injected(&dir);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded, commands);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_save_creates_missing_directory() {
        let dir = test_dir();
        let nested = format!("{}/nested/deeper", dir);
        save_injected(&nested, &[sample("my-tool")]).unwrap();
        assert!(injected_path(&nested).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_command_hit_and_miss() {
        let dir = test_dir();
        save_injected(&dir, &[sample("my-tool")]).unwrap();

        let found = find_command(&dir, "my-tool").unwrap();
        assert_eq!(found.command_name, "my-tool");
        assert_eq!(found.path, PathBuf::from("/usr/local/bin/my-tool"));
        assert!(find_command(&dir, "not-injected").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_upsert_adds_then_replaces() {
        let dir = test_dir();
        upsert_injected(&dir, sample("my-tool")).unwrap();
        assert_eq!(load_injected(&dir).len(), 1);

        let mut updated = sample("my-tool");
        updated.version = "2.0.0".to_string();
        updated.path = PathBuf::from("/opt/my-tool");
        upsert_injected(&dir, updated).unwrap();

        let loaded = load_injected(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].version, "2.0.0");
        assert_eq!(loaded[0].path, PathBuf::from("/opt/my-tool"));

        upsert_injected(&dir, sample("second-tool")).unwrap();
        assert_eq!(load_injected(&dir).len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_save_overwrites_previous_contents() {
        let dir = test_dir();
        save_injected(&dir, &[sample("a"), sample("b")]).unwrap();
        save_injected(&dir, &[sample("c")]).unwrap();

        let loaded = load_injected(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].command_name, "c");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_malformed_store_is_empty() {
        let dir = test_dir();
        std::fs::write(injected_path(&dir), b"{not json at all").unwrap();
        assert!(load_injected(&dir).is_empty());
        assert!(find_command(&dir, "my-tool").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_manifest_converts_to_injected_command() {
        let manifest = BallManifest {
            command_name: "my-tool".to_string(),
            description: "desc".to_string(),
            version: "1.2.3".to_string(),
            flags: vec!["--yes".to_string()],
            author: "HMythical".to_string(),
            require_root: true,
            depends: vec!["ffmpeg".to_string()],
            path: PathBuf::from("/usr/local/bin/my-tool"),
        };

        let injected: InjectedCommand = manifest.into();
        assert_eq!(injected.command_name, "my-tool");
        assert_eq!(injected.version, "1.2.3");
        assert!(injected.require_root);
        assert_eq!(injected.depends, vec!["ffmpeg"]);
    }

    #[test]
    fn test_resolve_baller_dir_from_config() {
        let dir = test_dir();
        std::fs::write(format!("{}/baller.conf", dir), "").unwrap();
        let config = BallerConfig::parse_config(&dir).unwrap();
        assert_eq!(
            resolve_baller_dir(&config),
            config
                .hooks_dir
                .parent()
                .unwrap()
                .to_string_lossy()
                .to_string()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
