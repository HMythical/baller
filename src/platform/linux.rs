use crate::error::error::BallError;
use crate::platform::common::PlatformManager;
use std::env;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

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

        if target.exists() {
            fs::remove_file(&target).map_err(BallError::FileIoErr)?;
        }

        symlink(source, &target).map_err(BallError::FileIoErr)?;
        Ok(())
    }

    fn remove_symlink(executable_name: &str) -> Result<(), BallError> {
        let install_dir = Self::get_install_dir()?;
        let target = install_dir.join(executable_name);
        if target.exists() {
            fs::remove_file(&target).map_err(BallError::FileIoErr)?;
        }
        Ok(())
    }
}
