use crate::error::error::BallError;
use crate::platform::common::PlatformManager;
use std::env;
use std::fs;
use std::os::windows::fs::symlink_file;
use std::path::PathBuf;

pub struct WindowsManager;

impl PlatformManager for WindowsManager {
    fn get_install_dir() -> Result<PathBuf, BallError> {
        let local_app_data = env::var("LOCALAPPDATA").map_err(|_| {
            BallError::FileIoErr(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "LOCALAPPDATA environment variable not set",
            ))
        })?;
        let mut path = PathBuf::from(local_app_data);
        path.push("baller");
        path.push("bin");
        Ok(path)
    }

    fn get_config_dir() -> Result<PathBuf, BallError> {
        let local_app_data = env::var("LOCALAPPDATA").map_err(|_| {
            BallError::FileIoErr(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "LOCALAPPDATA environment variable not set",
            ))
        })?;
        let mut path = PathBuf::from(local_app_data);
        path.push("baller");
        Ok(path)
    }

    fn create_symlink(source: &PathBuf, executable_name: &str) -> Result<(), BallError> {
        let install_dir = Self::get_install_dir()?;
        if !install_dir.exists() {
            fs::create_dir_all(&install_dir).map_err(BallError::FileIoErr)?;
        }

        // On Windows, executables should generally end with .exe
        let target_name = if executable_name.ends_with(".exe") {
            executable_name.to_string()
        } else {
            format!("{}.exe", executable_name)
        };

        let target = install_dir.join(target_name);

        if target.exists() {
            fs::remove_file(&target).map_err(BallError::FileIoErr)?;
        }

        // Use an actual local file copy instead of symlink_file here,
        // because symlinks on Windows require Developer Mode or Admin rights.
        fs::copy(source, &target).map_err(BallError::FileIoErr)?;
        Ok(())
    }

    fn remove_symlink(executable_name: &str) -> Result<(), BallError> {
        let install_dir = Self::get_install_dir()?;
        let target_name = if executable_name.ends_with(".exe") {
            executable_name.to_string()
        } else {
            format!("{}.exe", executable_name)
        };

        let target = install_dir.join(target_name);
        if target.exists() {
            fs::remove_file(&target).map_err(BallError::FileIoErr)?;
        }
        Ok(())
    }
}
