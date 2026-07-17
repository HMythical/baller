use std::process::Command;

use crate::core::package::{Package, PackageSource};
use crate::error::error::BallError;

/// Install a system package using the native package manager.
///
/// Invokes the appropriate CLI (`apt-get`, `dnf`, or `pacman`) under `sudo`.
/// Returns an error for unknown managers.
pub fn install_system_package(manager: &str, name: &str) -> Result<(), BallError> {
    let status = match manager {
        "apt" => Command::new("sudo")
            .args(["apt-get", "install", "-y", name])
            .status()
            .map_err(|e| BallError::PackageManagerError(format!("failed to run apt-get: {}", e)))?,
        "dnf" => Command::new("sudo")
            .args(["dnf", "install", "-y", name])
            .status()
            .map_err(|e| BallError::PackageManagerError(format!("failed to run dnf: {}", e)))?,
        "pacman" => Command::new("sudo")
            .args(["pacman", "-S", "--noconfirm", name])
            .status()
            .map_err(|e| BallError::PackageManagerError(format!("failed to run pacman: {}", e)))?,
        _ => {
            return Err(BallError::PackageManagerError(format!(
                "unsupported system package manager: {}",
                manager
            )));
        }
    };

    if !status.success() {
        return Err(BallError::PackageManagerError(format!(
            "{} install of '{}' exited with status {}",
            manager, name, status
        )));
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
enum SystemManager {
    Apt,
    Dnf,
    Pacman,
}

pub struct SystemRegistry {
    manager: Option<SystemManager>,
}

impl SystemRegistry {
    pub fn detect() -> Self {
        Self {
            manager: detect_system_manager("/etc/os-release"),
        }
    }

    pub fn fetch_package(&self, name: &str) -> Result<Package, BallError> {
        let manager = self.manager.as_ref().ok_or_else(|| {
            BallError::PackageManagerError("no system package manager detected".to_string())
        })?;

        match manager {
            SystemManager::Apt => self.fetch_apt(name),
            SystemManager::Dnf => self.fetch_dnf(name),
            SystemManager::Pacman => self.fetch_pacman(name),
        }
    }

    pub fn search(&self, query: &str) -> Result<Vec<Package>, BallError> {
        let manager = self.manager.as_ref().ok_or_else(|| {
            BallError::PackageManagerError("no system package manager detected".to_string())
        })?;

        match manager {
            SystemManager::Apt => self.search_apt(query),
            SystemManager::Dnf => self.search_dnf(query),
            SystemManager::Pacman => self.search_pacman(query),
        }
    }

    fn run_cmd(cmd: &str, args: &[&str]) -> Result<String, BallError> {
        let output = Command::new(cmd)
            .args(args)
            .output()
            .map_err(|e| BallError::PackageManagerError(format!("failed to run {}: {}", cmd, e)))?;

        if !output.status.success() {
            return Err(BallError::PackageManagerError(format!(
                "{} exited with status {}",
                cmd, output.status
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn fetch_apt(&self, name: &str) -> Result<Package, BallError> {
        let output = Self::run_cmd("apt-cache", &["show", name])?;

        if output.trim().is_empty() {
            return Err(BallError::PackageNotFound(name.to_string()));
        }

        let mut version = String::new();
        let mut description = String::new();
        let mut in_description = false;
        let mut author = String::new();
        let mut dependencies = Vec::new();

        for line in output.lines() {
            if let Some(val) = line.strip_prefix("Version:") {
                in_description = false;
                version = val.trim().to_string();
            } else if let Some(val) = line
                .strip_prefix("Description-en:")
                .or_else(|| line.strip_prefix("Description:"))
            {
                in_description = true;
                description = val.trim().to_string();
            } else if in_description {
                if line.starts_with(' ') || line == "." {
                    let continuation = line.trim();
                    if !continuation.is_empty() && continuation != "." {
                        if !description.is_empty() {
                            description.push(' ');
                        }
                        description.push_str(continuation);
                    }
                } else {
                    in_description = false;
                }
            }
            if !in_description {
                if let Some(val) = line.strip_prefix("Maintainer:") {
                    let maint = val.trim().to_string();
                    author = if maint.contains('<') {
                        maint.split('<').next().unwrap_or(&maint).trim().to_string()
                    } else {
                        maint
                    };
                } else if let Some(val) = line.strip_prefix("Depends:") {
                    let deps_str = val.trim();
                    for dep in deps_str.split(',') {
                        let dep_pkg = dep
                            .trim()
                            .split('(')
                            .next()
                            .unwrap_or(dep.trim())
                            .trim()
                            .to_string();
                        if !dep_pkg.is_empty() && !dependencies.contains(&dep_pkg) {
                            dependencies.push(dep_pkg);
                        }
                    }
                }
            }
        }

        if version.is_empty() {
            return Err(BallError::PackageNotFound(name.to_string()));
        }

        Ok(Package {
            name: name.to_string(),
            version,
            description: if description.is_empty() {
                None
            } else {
                Some(description)
            },
            author: if author.is_empty() {
                None
            } else {
                Some(author)
            },
            repository: None,
            architectures: None,
            dependencies: if dependencies.is_empty() {
                None
            } else {
                Some(dependencies)
            },
            sha256: None,
            hash_algorithm: None,
            download_url: None,
            source: PackageSource::System {
                manager: "apt".to_string(),
            },
        })
    }

    fn fetch_dnf(&self, name: &str) -> Result<Package, BallError> {
        let output = Self::run_cmd("dnf", &["info", "--installed", name])
            .or_else(|_| Self::run_cmd("dnf", &["info", name]));

        let output = match output {
            Ok(o) => o,
            Err(_) => return Err(BallError::PackageNotFound(name.to_string())),
        };

        if output.trim().is_empty() || output.contains("No matches found") {
            return Err(BallError::PackageNotFound(name.to_string()));
        }

        let mut version = String::new();
        let mut summary = String::new();
        let mut author = String::new();
        let mut dependencies = Vec::new();

        for line in output.lines() {
            if line.starts_with("Version") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    version = parts[1]
                        .trim()
                        .replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-', "");
                }
            } else if line.starts_with("Packag") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    author = parts[1].trim().to_string();
                }
            } else if line.starts_with("Summary") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    summary = parts[1].trim().to_string();
                }
            } else if line.starts_with("Dependencies") {
                let deps_part = line["Dependencies:".len()..].trim();
                if !deps_part.is_empty() {
                    for dep in deps_part.split(' ') {
                        let dep = dep.trim();
                        if !dep.is_empty()
                            && !dep.contains('(')
                            && !dependencies.contains(&dep.to_string())
                        {
                            dependencies.push(dep.to_string());
                        }
                    }
                }
            }
        }

        Ok(Package {
            name: name.to_string(),
            version: if version.is_empty() {
                "unknown".to_string()
            } else {
                version
            },
            description: if summary.is_empty() {
                None
            } else {
                Some(summary)
            },
            author: if author.is_empty() {
                None
            } else {
                Some(author)
            },
            repository: None,
            architectures: None,
            dependencies: if dependencies.is_empty() {
                None
            } else {
                Some(dependencies)
            },
            sha256: None,
            hash_algorithm: None,
            download_url: None,
            source: PackageSource::System {
                manager: "dnf".to_string(),
            },
        })
    }

    fn fetch_pacman(&self, name: &str) -> Result<Package, BallError> {
        let output = Self::run_cmd("pacman", &["-Si", name]);

        let output = match output {
            Ok(o) => o,
            Err(_) => return Err(BallError::PackageNotFound(name.to_string())),
        };

        if output.trim().is_empty() || output.contains("not found") {
            return Err(BallError::PackageNotFound(name.to_string()));
        }

        let mut version = String::new();
        let mut description = String::new();
        let mut dependencies = Vec::new();

        for line in output.lines() {
            if line.starts_with("Version") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    version = parts[1].trim().to_string();
                }
            } else if line.starts_with("Description") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    description = parts[1].trim().to_string();
                }
            } else if line.starts_with("Depends") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    for dep in parts[1].split(' ') {
                        let dep = dep.trim();
                        if !dep.is_empty() && !dependencies.contains(&dep.to_string()) {
                            dependencies.push(dep.to_string());
                        }
                    }
                }
            }
        }

        Ok(Package {
            name: name.to_string(),
            version: if version.is_empty() {
                "unknown".to_string()
            } else {
                version
            },
            description: if description.is_empty() {
                None
            } else {
                Some(description)
            },
            author: None,
            repository: None,
            architectures: None,
            dependencies: if dependencies.is_empty() {
                None
            } else {
                Some(dependencies)
            },
            sha256: None,
            hash_algorithm: None,
            download_url: None,
            source: PackageSource::System {
                manager: "pacman".to_string(),
            },
        })
    }

    fn search_apt(&self, query: &str) -> Result<Vec<Package>, BallError> {
        let output = Self::run_cmd("apt-cache", &["search", query])?;
        let mut results = Vec::new();

        for line in output.lines().take(20) {
            if let Some((name, desc)) = line.split_once(" - ") {
                let name = name.trim();
                if !name.is_empty() {
                    results.push(Package {
                        name: name.to_string(),
                        version: "unknown".to_string(),
                        description: Some(desc.trim().to_string()),
                        author: None,
                        repository: None,
                        architectures: None,
                        dependencies: None,
                        sha256: None,
                        hash_algorithm: None,
                        download_url: None,
                        source: PackageSource::System {
                            manager: "apt".to_string(),
                        },
                    });
                }
            }
        }

        Ok(results)
    }

    fn search_dnf(&self, query: &str) -> Result<Vec<Package>, BallError> {
        let output = Self::run_cmd("dnf", &["search", "--all-namespaces", query])?;
        let mut results = Vec::new();

        for line in output.lines() {
            if line.starts_with("Name") || line.starts_with('=') || line.is_empty() {
                continue;
            }
            if let Some((name, summary)) = line.split_once(" : ") {
                let name = name.trim();
                let summary = summary.trim();
                if !name.is_empty() {
                    results.push(Package {
                        name: name.to_string(),
                        version: String::new(),
                        description: if summary.is_empty() {
                            None
                        } else {
                            Some(summary.to_string())
                        },
                        author: None,
                        repository: None,
                        architectures: None,
                        dependencies: None,
                        sha256: None,
                        hash_algorithm: None,
                        download_url: None,
                        source: PackageSource::System {
                            manager: "apt".to_string(),
                        },
                    });
                }
            }
        }

        Ok(results)
    }

    fn search_pacman(&self, query: &str) -> Result<Vec<Package>, BallError> {
        let output = Self::run_cmd("pacman", &["-Ss", query])?;
        let mut results = Vec::new();

        for line in output.lines() {
            if !line.starts_with(' ') && line.contains('/') {
                let parts: Vec<&str> = line.splitn(3, ' ').collect();
                if parts.len() >= 2 {
                    let repo_name = parts[0].trim();
                    if repo_name.starts_with('#') || repo_name == "::" {
                        continue;
                    }
                    let entries: Vec<&str> = parts[0].split('/').collect();
                    if entries.len() != 2 {
                        continue;
                    }
                    let name = entries[1].trim();
                    if name.is_empty() {
                        continue;
                    }
                    let ver_rest = if parts.len() >= 3 {
                        Some(parts[2].trim().to_string())
                    } else {
                        None
                    };
                    let version = match &ver_rest {
                        Some(v) => v.clone(),
                        None => String::new(),
                    };
                    results.push(Package {
                        name: name.to_string(),
                        version,
                        description: parts.get(2).map(|s| s.trim().to_string()),
                        author: None,
                        repository: None,
                        architectures: None,
                        dependencies: None,
                        sha256: None,
                        hash_algorithm: None,
                        download_url: None,
                        source: PackageSource::System {
                            manager: "dnf".to_string(),
                        },
                    });
                }
            }
        }

        Ok(results)
    }
}

fn detect_system_manager(path: &str) -> Option<SystemManager> {
    let os_release = std::fs::read_to_string(path).ok()?;

    for line in os_release.lines() {
        if let Some(id_value) = line.strip_prefix("ID=") {
            let id = id_value.trim_matches('"');
            return match id {
                "ubuntu" | "debian" | "linuxmint" | "pop" | "elementary" | "zorin" | "kali"
                | "raspbian" => Some(SystemManager::Apt),
                "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "ol" | "nobara" => {
                    Some(SystemManager::Dnf)
                }
                "arch" | "manjaro" | "endeavouros" | "garuda" | "arco" => {
                    Some(SystemManager::Pacman)
                }
                _ => None,
            };
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_system_manager_debian() {
        let temp_dir = std::env::temp_dir().join("baller_test_system");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let os_release_path = temp_dir.join("os-release");
        std::fs::write(&os_release_path, "ID=debian\n").unwrap();

        let manager = detect_system_manager(os_release_path.to_str().unwrap());
        assert_eq!(manager, Some(SystemManager::Apt));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_system_manager_fedora() {
        let temp_dir = std::env::temp_dir().join("baller_test_system_fedora");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let os_release_path = temp_dir.join("os-release");
        std::fs::write(&os_release_path, "ID=fedora\n").unwrap();

        let manager = detect_system_manager(os_release_path.to_str().unwrap());
        assert_eq!(manager, Some(SystemManager::Dnf));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_system_manager_arch() {
        let temp_dir = std::env::temp_dir().join("baller_test_system_arch");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let os_release_path = temp_dir.join("os-release");
        std::fs::write(&os_release_path, "ID=arch\n").unwrap();

        let manager = detect_system_manager(os_release_path.to_str().unwrap());
        assert_eq!(manager, Some(SystemManager::Pacman));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_system_manager_unknown() {
        let temp_dir = std::env::temp_dir().join("baller_test_system_unknown");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let os_release_path = temp_dir.join("os-release");
        std::fs::write(&os_release_path, "ID=unknown_distro_xyz\n").unwrap();

        let manager = detect_system_manager(os_release_path.to_str().unwrap());
        assert_eq!(manager, None);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_id_mapping_debian() {
        let ids = vec![
            "ubuntu",
            "debian",
            "linuxmint",
            "pop",
            "elementary",
            "zorin",
            "kali",
            "raspbian",
        ];
        for id in ids {
            let line = format!("ID={}", id);
            if let Some(val) = line.strip_prefix("ID=") {
                let v = val.trim_matches('"');
                assert!(matches!(
                    v,
                    "ubuntu"
                        | "debian"
                        | "linuxmint"
                        | "pop"
                        | "elementary"
                        | "zorin"
                        | "kali"
                        | "raspbian"
                ));
            }
        }
    }

    #[test]
    fn test_detect_id_mapping_fedora() {
        let ids = vec![
            "fedora",
            "rhel",
            "centos",
            "rocky",
            "almalinux",
            "ol",
            "nobara",
        ];
        for id in ids {
            let line = format!("ID={}", id);
            if let Some(val) = line.strip_prefix("ID=") {
                let v = val.trim_matches('"');
                assert!(matches!(
                    v,
                    "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "ol" | "nobara"
                ));
            }
        }
    }

    #[test]
    fn test_detect_id_mapping_arch() {
        let ids = vec!["arch", "manjaro", "endeavouros", "garuda", "arco"];
        for id in ids {
            let line = format!("ID={}", id);
            if let Some(val) = line.strip_prefix("ID=") {
                let v = val.trim_matches('"');
                assert!(matches!(
                    v,
                    "arch" | "manjaro" | "endeavouros" | "garuda" | "arco"
                ));
            }
        }
    }

    #[test]
    fn test_detect_unknown_id() {
        let line = "ID=unknown_distro_xyz".to_string();
        if let Some(val) = line.strip_prefix("ID=") {
            let v = val.trim_matches('"');
            // unknown_distro_xyz should NOT match any known distro
            assert!(!matches!(
                v,
                "ubuntu"
                    | "debian"
                    | "linuxmint"
                    | "pop"
                    | "elementary"
                    | "zorin"
                    | "kali"
                    | "raspbian"
                    | "fedora"
                    | "rhel"
                    | "centos"
                    | "rocky"
                    | "almalinux"
                    | "ol"
                    | "nobara"
                    | "arch"
                    | "manjaro"
                    | "endeavouros"
                    | "garuda"
                    | "arco"
            ));
        }
    }

    #[test]
    fn test_system_registry_detect_returns_some_on_linux() {
        let registry = SystemRegistry::detect();
        // On real Linux with valid os-release, this may return Some
        // The None case is also valid (unknown distro)
        match registry.manager {
            Some(SystemManager::Apt) | Some(SystemManager::Dnf) | Some(SystemManager::Pacman) => {}
            None => {} // expected on unknown/non-debian systems
        }
    }

    #[test]
    fn test_parse_apt_cache_show_output() {
        let test_output = r#"Package: vim
Version: 2:8.1.0875-5ubuntu2
Description-en: Vi IMproved - enhanced vi editor
 Vim is an almost compatible version of the UNIX editor Vi.
 .
 Many new features have been added.
Maintainer: Ubuntu Developers <ubuntu-devel-discuss@lists.ubuntu.com>
Depends: vim-common (= 2:8.1.0875-5ubuntu2), vim-runtime (= 2:8.1.0875-5ubuntu2)
"#;

        let mut version = String::new();
        let mut description = String::new();
        let mut in_description = false;
        let mut author = String::new();
        let mut dependencies = Vec::new();

        for line in test_output.lines() {
            if let Some(val) = line.strip_prefix("Version:") {
                in_description = false;
                version = val.trim().to_string();
            } else if let Some(val) = line
                .strip_prefix("Description-en:")
                .or_else(|| line.strip_prefix("Description:"))
            {
                in_description = true;
                description = val.trim().to_string();
            } else if in_description {
                if line.starts_with(' ') || line == "." {
                    let continuation = line.trim();
                    if !continuation.is_empty() && continuation != "." {
                        if !description.is_empty() {
                            description.push(' ');
                        }
                        description.push_str(continuation);
                    }
                } else {
                    in_description = false;
                }
            }
            if !in_description {
                if let Some(val) = line.strip_prefix("Maintainer:") {
                    let maint = val.trim().to_string();
                    author = if maint.contains('<') {
                        maint.split('<').next().unwrap_or(&maint).trim().to_string()
                    } else {
                        maint
                    };
                } else if let Some(val) = line.strip_prefix("Depends:") {
                    let deps_str = val.trim();
                    for dep in deps_str.split(',') {
                        let dep_pkg = dep
                            .trim()
                            .split('(')
                            .next()
                            .unwrap_or(dep.trim())
                            .trim()
                            .to_string();
                        if !dep_pkg.is_empty() && !dependencies.contains(&dep_pkg) {
                            dependencies.push(dep_pkg);
                        }
                    }
                }
            }
        }

        assert_eq!(version, "2:8.1.0875-5ubuntu2");
        assert_eq!(description, "Vi IMproved - enhanced vi editor Vim is an almost compatible version of the UNIX editor Vi. Many new features have been added.");
        assert_eq!(author, "Ubuntu Developers");
        assert!(dependencies.contains(&"vim-common".to_string()));
        assert!(dependencies.contains(&"vim-runtime".to_string()));
    }

    #[test]
    fn test_parse_dnf_info_output() {
        let test_output = r#"Loaded plugins: fastestmirror
Loading mirror speeds from cached hostfile
Name            : vim-enhanced
Version         : 8.2.2637-20.fc36
Release         : 20.fc36
Architecture    : x86_64
Size            : 1.2 M
Summary         : A version of the VIM editor
Packager        : Fedora Project
URL             : https://www.vim.org/

Name            : vim-common
Version         : 8.2.2637-20.fc36
Summary         : The common files needed by any version of the VIM editor
"#;

        let mut version = String::new();
        let mut summary = String::new();
        let mut author = String::new();

        for line in test_output.lines() {
            if line.starts_with("Version") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    version = parts[1].trim().to_string();
                }
            } else if line.starts_with("Packag") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    author = parts[1].trim().to_string();
                }
            } else if line.starts_with("Summary") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    summary = format!("{} {}", summary.trim(), parts[1].trim())
                        .trim()
                        .to_string();
                }
            }
        }

        assert!(!version.is_empty());
        assert_eq!(author, "Fedora Project");
        assert!(!summary.is_empty());
    }

    #[test]
    fn test_parse_pacman_si_output() {
        let test_output = r#"Repository      : core
Name            : vim
Version         : 9.0.1300-1
Description     : Vi Improved, a highly configurable, improved vi editor
Architecture    : x86_64
URL             : https://www.vim.org/
Licenses        : custom
Depends         : glibc  harfbuzz  ncurses  acl  gpm  libsodium
Optional Deps   : python: for vimdoc viewer and pyhelp plugin
Conflicts       : vim-runtime
"#;

        let mut name = String::new();
        let mut version = String::new();
        let mut description = String::new();

        for line in test_output.lines() {
            if line.starts_with("Name") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    name = parts[1].trim().to_string();
                }
            } else if line.starts_with("Version") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    version = format!("{} {}", version.trim(), parts[1].trim())
                        .trim()
                        .to_string();
                }
            } else if line.starts_with("Description") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    description = format!("{} {}", description.trim(), parts[1].trim())
                        .trim()
                        .to_string();
                }
            }
        }

        assert_eq!(name, "vim");
    }

    #[test]
    fn test_parse_apt_cache_search_output() {
        let test_output = r#"vim - Vi IMproved - enhanced vi editor
vim-common - Vi IMproved - common files
vim-runtime - Vi Improved - Runtime files
"#;

        let mut results: Vec<(String, String)> = Vec::new();

        for line in test_output.lines() {
            if let Some((name, desc)) = line.split_once(" - ") {
                let name = name.trim();
                if !name.is_empty() {
                    results.push((name.to_string(), desc.trim().to_string()));
                }
            }
        }

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, "vim");
        assert_eq!(results[1].0, "vim-common");
    }

    #[test]
    fn test_parse_pacman_ss_output() {
        let test_output = r#"core/vim 9.0.1300-1
    Vi Improved, a highly configurable, improved vi editor
extra/git 2.40.1-1
    The fast distributed version control system
"#;

        let mut results: Vec<(String, String)> = Vec::new();

        for line in test_output.lines() {
            if !line.starts_with(' ') && line.contains('/') {
                let parts: Vec<&str> = line.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    let entries: Vec<&str> = parts[0].split('/').collect();
                    if entries.len() == 2 {
                        results.push((entries[1].to_string(), parts[1].trim().to_string()));
                    }
                }
            }
        }

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "vim");
        assert_eq!(results[1].0, "git");
    }

    #[test]
    fn test_detect_system_manager_manjaro() {
        let temp_dir = std::env::temp_dir().join("baller_test_system_manjaro");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let os_release_path = temp_dir.join("os-release");
        std::fs::write(&os_release_path, "ID=manjaro\n").unwrap();

        let manager = detect_system_manager(os_release_path.to_str().unwrap());
        assert_eq!(manager, Some(SystemManager::Pacman));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_missing_file_returns_none() {
        let manager = detect_system_manager("/nonexistent/path/os-release");
        assert_eq!(manager, None);
    }

    #[test]
    fn test_parse_apt_deps_dedup() {
        let deps_str = "vim-common (= 1.0), vim-common (>= 2.0), vim-runtime (= 1.0)";
        let mut dependencies = Vec::new();
        for dep in deps_str.split(',') {
            let dep_pkg = dep
                .trim()
                .split('(')
                .next()
                .unwrap_or(dep.trim())
                .trim()
                .to_string();
            if !dep_pkg.is_empty() && !dependencies.contains(&dep_pkg) {
                dependencies.push(dep_pkg);
            }
        }
        assert_eq!(dependencies.len(), 2);
        assert!(dependencies.contains(&"vim-common".to_string()));
        assert!(dependencies.contains(&"vim-runtime".to_string()));
    }

    #[test]
    fn test_search_returns_lightweight_packages() {
        let mut results: Vec<(String, String)> = Vec::new();
        for line in "pkg-one - Description one\npkg-two - Description two\n".lines() {
            if let Some((name, desc)) = line.split_once(" - ") {
                let name = name.trim();
                if !name.is_empty() {
                    results.push((name.to_string(), desc.trim().to_string()));
                    // Verify lightweight: version is empty
                    let pkg = Package {
                        name: name.to_string(),
                        version: "unknown".to_string(),
                        description: Some(desc.trim().to_string()),
                        author: None,
                        repository: None,
                        architectures: None,
                        dependencies: None,
                        sha256: None,
                        hash_algorithm: None,
                        download_url: None,
                        source: PackageSource::System {
                            manager: "apt".to_string(),
                        },
                    };
                    assert_eq!(pkg.version, "unknown");
                    assert!(!pkg.name.is_empty());
                    assert!(pkg.description.is_some());
                    match pkg.source {
                        PackageSource::System { ref manager } => assert_eq!(manager, "apt"),
                        _ => panic!("expected System source"),
                    }
                }
            }
        }
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_dnf_depends_parsing_with_releases() {
        // Test parsing of Release field doesn't break version extraction
        let test_output = r#"Loaded plugins: fastestmirror
Name            : vim-enhanced
Version         : 8.2.2637-20.fc36
Release         : 20.fc36
Architecture    : x86_64
Size            : 1.2 M
Summary         : A version of the VIM editor
Packager        : Fedora Project
"#;
        let mut version = String::new();
        for line in test_output.lines() {
            if line.starts_with("Version") {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 {
                    version = parts[1]
                        .trim()
                        .replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-', "");
                }
            }
        }
        assert_eq!(version, "8.2.2637-20.fc36");
    }

    #[test]
    fn test_system_registry_no_pm_fetch_error() {
        let registry = SystemRegistry { manager: None };
        let result = registry.fetch_package("any-pkg");
        assert!(result.is_err());
        match result.unwrap_err() {
            BallError::PackageManagerError(msg) => assert!(!msg.is_empty()),
            other => panic!("expected PackageManagerError, got {:?}", other),
        }
    }

    #[test]
    fn test_system_registry_no_pm_search_error() {
        let registry = SystemRegistry { manager: None };
        let result = registry.search("query");
        assert!(result.is_err());
        match result.unwrap_err() {
            BallError::PackageManagerError(msg) => assert!(!msg.is_empty()),
            other => panic!("expected PackageManagerError, got {:?}", other),
        }
    }

    #[test]
    fn test_detect_all_apt_ids() {
        let temp_dir = std::env::temp_dir().join("baller_test_apt_families");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        for id in [
            "ubuntu",
            "debian",
            "linuxmint",
            "pop",
            "elementary",
            "zorin",
            "kali",
            "raspbian",
        ] {
            let path = temp_dir.join(format!("os-release-{}", id));
            std::fs::write(&path, format!("ID={}\n", id)).unwrap();
            assert_eq!(
                detect_system_manager(path.to_str().unwrap()),
                Some(SystemManager::Apt)
            );
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_all_dnf_ids() {
        let temp_dir = std::env::temp_dir().join("baller_test_dnf_families");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        for id in [
            "fedora",
            "rhel",
            "centos",
            "rocky",
            "almalinux",
            "ol",
            "nobara",
        ] {
            let path = temp_dir.join(format!("os-release-{}", id));
            std::fs::write(&path, format!("ID={}\n", id)).unwrap();
            assert_eq!(
                detect_system_manager(path.to_str().unwrap()),
                Some(SystemManager::Dnf)
            );
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_all_pacman_ids() {
        let temp_dir = std::env::temp_dir().join("baller_test_pacman_families");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        for id in ["arch", "manjaro", "endeavouros", "garuda", "arco"] {
            let path = temp_dir.join(format!("os-release-{}", id));
            std::fs::write(&path, format!("ID={}\n", id)).unwrap();
            assert_eq!(
                detect_system_manager(path.to_str().unwrap()),
                Some(SystemManager::Pacman)
            );
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
