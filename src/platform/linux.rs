use crate::error::error::BallError;
use crate::platform::common::PlatformManager;
use std::env;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

pub struct LinuxManager;

impl PlatformManager for LinuxManager {
    fn get_install_dir() -> Result<PathBuf, BallError> {
        let home = env::var("HOME").map_err(|_| {
            BallError::FileIoErr(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME environment variable not set",
            ))
        })?;
        let mut path = PathBuf::from(home);
        path.push(".local");
        path.push("bin");
        Ok(path)
    }

    fn get_config_dir() -> Result<PathBuf, BallError> {
        let home = env::var("HOME").map_err(|_| {
            BallError::FileIoErr(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME environment variable not set",
            ))
        })?;
        let mut path = PathBuf::from(home);
        path.push(".baller");
        Ok(path)
    }

    fn create_symlink_in(
        install_dir: &std::path::Path,
        source: &std::path::Path,
        executable_name: &str,
    ) -> Result<(), BallError> {
        if !install_dir.exists() {
            fs::create_dir_all(install_dir).map_err(BallError::FileIoErr)?;
        }
        let target = install_dir.join(executable_name);

        remove_link(&target)?;

        symlink(source, &target).map_err(BallError::FileIoErr)?;
        Ok(())
    }

    fn remove_symlink(executable_name: &str) -> Result<(), BallError> {
        let install_dir = Self::get_install_dir()?;
        remove_link(&install_dir.join(executable_name))
    }
}

/// Whether anything occupies `path`, a dangling symlink included.
///
/// `Path::exists` follows symlinks, so it reports a link whose target is gone
/// — after `update` prunes an old extract dir, or `sweep --all` — as absent.
/// Replacing or removing a managed link has to see it anyway (#38).
fn occupied(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Remove the managed link at `target`, live or dangling; absent is fine
fn remove_link(target: &Path) -> Result<(), BallError> {
    if occupied(target) {
        fs::remove_file(target).map_err(BallError::FileIoErr)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = env::temp_dir().join(format!(
            "baller_test_linux_links_{}_{}_{}",
            tag,
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_dangling_link_counts_as_occupied() {
        let dir = test_dir("occupied");
        let link = dir.join("tool");
        symlink(dir.join("gone"), &link).unwrap();

        assert!(!link.exists(), "precondition: exists() follows the link");
        assert!(occupied(&link));
        assert!(!occupied(&dir.join("nothing-here")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_create_symlink_in_replaces_a_dangling_link() {
        let dir = test_dir("replace_dangling");
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        symlink(dir.join("pruned-1.0.0/tool"), bin.join("tool")).unwrap();

        let source = dir.join("tool-2.0.0");
        fs::write(&source, b"v2").unwrap();

        // Was `File exists (os error 17)`
        LinuxManager::create_symlink_in(&bin, &source, "tool").unwrap();
        assert_eq!(fs::read_link(bin.join("tool")).unwrap(), source);
        assert_eq!(fs::read(bin.join("tool")).unwrap(), b"v2");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_create_symlink_in_replaces_a_live_link_and_creates_missing_dir() {
        let dir = test_dir("replace_live");
        let bin = dir.join("bin");
        let old = dir.join("old");
        let new = dir.join("new");
        fs::write(&old, b"old").unwrap();
        fs::write(&new, b"new").unwrap();

        LinuxManager::create_symlink_in(&bin, &old, "tool").unwrap();
        LinuxManager::create_symlink_in(&bin, &new, "tool").unwrap();
        assert_eq!(fs::read_link(bin.join("tool")).unwrap(), new);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remove_link_removes_dangling_and_live_links() {
        let dir = test_dir("remove");
        let dangling = dir.join("dangling");
        symlink(dir.join("gone"), &dangling).unwrap();
        let target = dir.join("target");
        fs::write(&target, b"x").unwrap();
        let live = dir.join("live");
        symlink(&target, &live).unwrap();

        // Was silently left behind while eject reported success
        remove_link(&dangling).unwrap();
        assert!(!occupied(&dangling));
        remove_link(&live).unwrap();
        assert!(!occupied(&live));
        assert!(
            target.exists(),
            "only the link is removed, never its target"
        );
        remove_link(&dir.join("absent")).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
