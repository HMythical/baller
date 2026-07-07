pub mod common;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub mod windows;

#[allow(unused_imports)]
#[cfg(target_os = "linux")]
pub use linux::LinuxManager as ActivePlatformManager;

#[allow(unused_imports)]
#[cfg(target_os = "windows")]
pub use windows::WindowsManager as ActivePlatformManager;
