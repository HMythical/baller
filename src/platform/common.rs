use crate::error::error::BallError;
use std::path::{Path, PathBuf};

pub trait PlatformManager {
    /// Gets the global installation directory for placing binaries.
    fn get_install_dir() -> Result<PathBuf, BallError>;

    /// Gets the global configuration directory for baller settings.
    fn get_config_dir() -> Result<PathBuf, BallError>;

    /// Gets the path to the SQLite local database.
    #[allow(dead_code)]
    fn get_db_path() -> Result<PathBuf, BallError> {
        let mut path = Self::get_config_dir()?;
        path.push("db");
        path.push("baller.db");
        Ok(path)
    }

    /// Exposes a package binary inside a specific directory.
    fn create_symlink_in(
        install_dir: &Path,
        source: &Path,
        executable_name: &str,
    ) -> Result<(), BallError>;

    /// Exposes a package binary by creating a system-appropriate symlink/wrapper.
    fn create_symlink(source: &Path, executable_name: &str) -> Result<(), BallError> {
        Self::create_symlink_in(&Self::get_install_dir()?, source, executable_name)
    }

    /// Removes an exposed package binary.
    fn remove_symlink(executable_name: &str) -> Result<(), BallError>;
}
