use crate::error::error::BallError;
use std::path::PathBuf;

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

    /// Exposes a package binary by creating a system-appropriate symlink/wrapper.
    fn create_symlink(source: &PathBuf, executable_name: &str) -> Result<(), BallError>;

    /// Removes an exposed package binary.
    fn remove_symlink(executable_name: &str) -> Result<(), BallError>;
}
