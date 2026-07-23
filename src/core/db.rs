use rusqlite::{params, Connection};
use std::path::PathBuf;

use crate::core::package::{Package, PackageSource};
use crate::error::error::BallError;
use crate::platform::common::PlatformManager;
use crate::utils::fs::ensure_dir;

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

#[derive(Debug, Clone)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub source: String,
    pub source_detail: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    #[allow(dead_code)]
    pub repository: Option<String>,
    #[allow(dead_code)]
    pub download_url: Option<String>,
    #[allow(dead_code)]
    pub sha256: Option<String>,
    pub frozen: bool,
    pub user_installed: bool,
    pub install_path: String,
    pub bin_path: Option<String>,
    #[allow(dead_code)]
    pub manifest_path: Option<String>,
    #[allow(dead_code)]
    pub installed_at: String,
    #[allow(dead_code)]
    pub dependencies: Vec<String>,
}

pub struct DbManager {
    conn: Connection,
}

impl DbManager {
    #[allow(dead_code)]
    pub fn init() -> Result<Self, BallError> {
        let db_path = ActiveManager::get_db_path()?;
        Self::init_at_path(&db_path)
    }

    pub fn init_at_path(db_path: &PathBuf) -> Result<Self, BallError> {
        if let Some(parent) = db_path.parent() {
            ensure_dir(parent)?;
        }

        let conn = Connection::open(db_path).map_err(|e| {
            BallError::InvalidConfig(format!(
                "failed to open database at {}: {}",
                db_path.display(),
                e
            ))
        })?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| BallError::InvalidConfig(format!("failed to set pragmas: {}", e)))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS installed_packages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                version TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'github',
                source_detail TEXT,
                description TEXT,
                author TEXT,
                repository TEXT,
                download_url TEXT,
                sha256 TEXT,
                frozen BOOLEAN NOT NULL DEFAULT 0,
                user_installed BOOLEAN NOT NULL DEFAULT 1,
                install_path TEXT NOT NULL,
                bin_path TEXT,
                manifest_path TEXT,
                installed_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS package_dependencies (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pkg_name TEXT NOT NULL,
                dep_name TEXT NOT NULL,
                dep_version TEXT NOT NULL,
                FOREIGN KEY (pkg_name) REFERENCES installed_packages(name) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS lockfile (
                name TEXT PRIMARY KEY,
                version TEXT NOT NULL,
                source TEXT NOT NULL,
                sha256 TEXT
            );",
        )
        .map_err(|e| BallError::InvalidConfig(format!("failed to create schema: {}", e)))?;

        // Migration: add user_installed column to existing databases that don't have it
        let has_column: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('installed_packages') WHERE name='user_installed'",
            [],
            |row| row.get(0),
        ).unwrap_or(0);
        if has_column == 0 {
            let _ = conn.execute(
                "ALTER TABLE installed_packages ADD COLUMN user_installed BOOLEAN NOT NULL DEFAULT 1",
                [],
            );
            // Backfill: entries without the column get default value of 1
        }

        Ok(Self { conn })
    }

    pub fn insert_package(
        &self,
        pkg: &Package,
        install_path: &str,
        bin_path: Option<&str>,
        manifest_path: Option<&str>,
        user_installed: bool,
    ) -> Result<(), BallError> {
        let (source, source_detail) = serialize_source(&pkg.source);

        self.conn.execute(
            "INSERT INTO installed_packages (name, version, source, source_detail, description, author, repository, download_url, sha256, user_installed, install_path, bin_path, manifest_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(name) DO UPDATE SET
                 version=excluded.version,
                 source=excluded.source,
                 source_detail=excluded.source_detail,
                 description=excluded.description,
                 author=excluded.author,
                 repository=excluded.repository,
                 download_url=excluded.download_url,
                 sha256=excluded.sha256,
                 user_installed=excluded.user_installed,
                 install_path=excluded.install_path,
                 bin_path=excluded.bin_path,
                 manifest_path=excluded.manifest_path,
                 installed_at=datetime('now')",
            params![
                pkg.name, pkg.version, source, source_detail,
                pkg.description, pkg.author, pkg.repository,
                pkg.download_url, pkg.sha256, user_installed, install_path, bin_path, manifest_path
            ],
        ).map_err(|e| BallError::InvalidConfig(format!("failed to insert package '{}': {}", pkg.name, e)))?;

        if let Some(deps) = &pkg.dependencies {
            self.conn
                .execute(
                    "DELETE FROM package_dependencies WHERE pkg_name = ?1",
                    params![pkg.name],
                )
                .map_err(|e| {
                    BallError::InvalidConfig(format!(
                        "failed to clear deps for '{}': {}",
                        pkg.name, e
                    ))
                })?;

            for dep in deps {
                let dep_parts: Vec<&str> = dep.split('>').collect();
                let dep_name = dep_parts[0].trim();
                let dep_version = dep_parts
                    .get(1)
                    .map(|s| s.trim_start_matches('=').trim())
                    .unwrap_or("latest");

                self.conn.execute(
                    "INSERT INTO package_dependencies (pkg_name, dep_name, dep_version) VALUES (?1, ?2, ?3)",
                    params![pkg.name, dep_name, dep_version],
                ).map_err(|e| BallError::InvalidConfig(format!("failed to insert dep for '{}': {}", pkg.name, e)))?;
            }
        }

        Ok(())
    }

    pub fn remove_package(&self, name: &str) -> Result<(), BallError> {
        self.conn
            .execute(
                "DELETE FROM package_dependencies WHERE pkg_name = ?1",
                params![name],
            )
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to remove deps for '{}': {}", name, e))
            })?;

        let affected = self
            .conn
            .execute(
                "DELETE FROM installed_packages WHERE name = ?1",
                params![name],
            )
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to remove package '{}': {}", name, e))
            })?;

        if affected == 0 {
            return Err(BallError::PackageNotFound(name.to_string()));
        }

        Ok(())
    }

    pub fn get_package(&self, name: &str) -> Result<InstalledPackage, BallError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, source, source_detail, description, author, repository,
                    download_url, sha256, frozen, user_installed, install_path, bin_path, manifest_path, installed_at
             FROM installed_packages WHERE name = ?1"
        ).map_err(|e| BallError::InvalidConfig(format!("query error: {}", e)))?;

        let pkg = stmt
            .query_row(params![name], |row| {
                Ok(InstalledPackage {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    source: row.get(2)?,
                    source_detail: row.get(3)?,
                    description: row.get(4)?,
                    author: row.get(5)?,
                    repository: row.get(6)?,
                    download_url: row.get(7)?,
                    sha256: row.get(8)?,
                    frozen: row.get(9)?,
                    user_installed: row.get(10)?,
                    install_path: row.get(11)?,
                    bin_path: row.get(12)?,
                    manifest_path: row.get(13)?,
                    installed_at: row.get(14)?,
                    dependencies: Vec::new(),
                })
            })
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    BallError::PackageNotFound(name.to_string())
                }
                _ => BallError::InvalidConfig(format!("failed to query package '{}': {}", name, e)),
            })?;

        Ok(pkg)
    }

    pub fn list_packages(&self) -> Result<Vec<InstalledPackage>, BallError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, source, source_detail, description, author, repository,
                    download_url, sha256, frozen, user_installed, install_path, bin_path, manifest_path, installed_at
             FROM installed_packages ORDER BY name"
        ).map_err(|e| BallError::InvalidConfig(format!("query error: {}", e)))?;

        let pkgs = stmt
            .query_map([], |row| {
                Ok(InstalledPackage {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    source: row.get(2)?,
                    source_detail: row.get(3)?,
                    description: row.get(4)?,
                    author: row.get(5)?,
                    repository: row.get(6)?,
                    download_url: row.get(7)?,
                    sha256: row.get(8)?,
                    frozen: row.get(9)?,
                    user_installed: row.get(10)?,
                    install_path: row.get(11)?,
                    bin_path: row.get(12)?,
                    manifest_path: row.get(13)?,
                    installed_at: row.get(14)?,
                    dependencies: Vec::new(),
                })
            })
            .map_err(|e| BallError::InvalidConfig(format!("failed to list packages: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| BallError::InvalidConfig(format!("failed to collect packages: {}", e)))?;

        Ok(pkgs)
    }

    #[allow(dead_code)]
    pub fn search_installed(&self, query: &str) -> Result<Vec<InstalledPackage>, BallError> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT name, version, source, source_detail, description, author, repository,
                    download_url, sha256, frozen, user_installed, install_path, bin_path, manifest_path, installed_at
             FROM installed_packages WHERE name LIKE ?1 OR description LIKE ?1 ORDER BY name"
        ).map_err(|e| BallError::InvalidConfig(format!("query error: {}", e)))?;

        let pkgs = stmt
            .query_map(params![pattern], |row| {
                Ok(InstalledPackage {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    source: row.get(2)?,
                    source_detail: row.get(3)?,
                    description: row.get(4)?,
                    author: row.get(5)?,
                    repository: row.get(6)?,
                    download_url: row.get(7)?,
                    sha256: row.get(8)?,
                    frozen: row.get(9)?,
                    user_installed: row.get(10)?,
                    install_path: row.get(11)?,
                    bin_path: row.get(12)?,
                    manifest_path: row.get(13)?,
                    installed_at: row.get(14)?,
                    dependencies: Vec::new(),
                })
            })
            .map_err(|e| BallError::InvalidConfig(format!("failed to search packages: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to collect search results: {}", e))
            })?;

        Ok(pkgs)
    }

    #[allow(dead_code)]
    pub fn package_count(&self) -> Result<i64, BallError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM installed_packages", [], |row| {
                row.get(0)
            })
            .map_err(|e| BallError::InvalidConfig(format!("failed to count packages: {}", e)))?;
        Ok(count)
    }

    #[allow(dead_code)]
    pub fn package_exists(&self, name: &str) -> Result<bool, BallError> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .map_err(|e| BallError::InvalidConfig(format!("failed to check package: {}", e)))?;
        Ok(count > 0)
    }

    pub fn is_frozen(&self, name: &str) -> Result<bool, BallError> {
        let frozen: bool = self
            .conn
            .query_row(
                "SELECT frozen FROM installed_packages WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .unwrap_or(false);
        Ok(frozen)
    }

    pub fn set_frozen(&self, name: &str, freeze: bool) -> Result<(), BallError> {
        let affected = self
            .conn
            .execute(
                "UPDATE installed_packages SET frozen = ?1 WHERE name = ?2",
                params![freeze, name],
            )
            .map_err(|e| BallError::InvalidConfig(format!("failed to set frozen: {}", e)))?;

        if affected == 0 {
            return Err(BallError::PackageNotFound(name.to_string()));
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn list_frozen(&self) -> Result<Vec<InstalledPackage>, BallError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, source, source_detail, description, author, repository,
                    download_url, sha256, frozen, user_installed, install_path, bin_path, manifest_path, installed_at
             FROM installed_packages WHERE frozen = 1 ORDER BY name"
        ).map_err(|e| BallError::InvalidConfig(format!("query error: {}", e)))?;

        let pkgs = stmt
            .query_map([], |row| {
                Ok(InstalledPackage {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    source: row.get(2)?,
                    source_detail: row.get(3)?,
                    description: row.get(4)?,
                    author: row.get(5)?,
                    repository: row.get(6)?,
                    download_url: row.get(7)?,
                    sha256: row.get(8)?,
                    frozen: row.get(9)?,
                    user_installed: row.get(10)?,
                    install_path: row.get(11)?,
                    bin_path: row.get(12)?,
                    manifest_path: row.get(13)?,
                    installed_at: row.get(14)?,
                    dependencies: Vec::new(),
                })
            })
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to list frozen packages: {}", e))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to collect frozen packages: {}", e))
            })?;

        Ok(pkgs)
    }

    pub fn get_dependencies(&self, pkg_name: &str) -> Result<Vec<(String, String)>, BallError> {
        let mut stmt = self.conn.prepare(
            "SELECT dep_name, dep_version FROM package_dependencies WHERE pkg_name = ?1 ORDER BY dep_name"
        ).map_err(|e| BallError::InvalidConfig(format!("query error: {}", e)))?;

        let deps = stmt
            .query_map(params![pkg_name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| BallError::InvalidConfig(format!("failed to query dependencies: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to collect dependencies: {}", e))
            })?;

        Ok(deps)
    }

    /// Check if a package is listed as a dependency by any other installed package.
    pub fn is_depended_on(&self, pkg_name: &str) -> Result<bool, BallError> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM package_dependencies WHERE dep_name = ?1",
                params![pkg_name],
                |row| row.get(0),
            )
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to query deps for '{}': {}", pkg_name, e))
            })?;
        Ok(count > 0)
    }

    #[allow(dead_code)]
    pub fn insert_lock_entry(
        &self,
        name: &str,
        version: &str,
        source: &str,
        sha256: Option<&str>,
    ) -> Result<(), BallError> {
        self.conn.execute(
            "INSERT INTO lockfile (name, version, source, sha256) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET version=excluded.version, source=excluded.source, sha256=excluded.sha256",
            params![name, version, source, sha256],
        ).map_err(|e| BallError::InvalidConfig(format!("failed to insert lock entry: {}", e)))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_lock_entry(
        &self,
        name: &str,
    ) -> Result<(String, String, Option<String>), BallError> {
        let result = self
            .conn
            .query_row(
                "SELECT version, source, sha256 FROM lockfile WHERE name = ?1",
                params![name],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    BallError::PackageNotFound(name.to_string())
                }
                _ => BallError::InvalidConfig(format!("failed to query lock entry: {}", e)),
            })?;

        Ok(result)
    }

    #[allow(dead_code)]
    #[allow(clippy::type_complexity)]
    pub fn get_all_lock_entries(
        &self,
    ) -> Result<Vec<(String, String, String, Option<String>)>, BallError> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, version, source, sha256 FROM lockfile ORDER BY name")
            .map_err(|e| BallError::InvalidConfig(format!("query error: {}", e)))?;

        let entries = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|e| BallError::InvalidConfig(format!("failed to query lock entries: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to collect lock entries: {}", e))
            })?;

        Ok(entries)
    }

    #[allow(dead_code)]
    pub fn remove_lock_entry(&self, name: &str) -> Result<(), BallError> {
        self.conn
            .execute("DELETE FROM lockfile WHERE name = ?1", params![name])
            .map_err(|e| BallError::InvalidConfig(format!("failed to remove lock entry: {}", e)))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn sync_lockfile_from_installed(&self) -> Result<(), BallError> {
        let pkgs = self.list_packages()?;

        self.conn
            .execute("DELETE FROM lockfile", [])
            .map_err(|e| BallError::InvalidConfig(format!("failed to clear lockfile: {}", e)))?;

        for pkg in &pkgs {
            self.conn
                .execute(
                    "INSERT INTO lockfile (name, version, source, sha256) VALUES (?1, ?2, ?3, ?4)",
                    params![pkg.name, pkg.version, pkg.source, pkg.sha256],
                )
                .map_err(|e| {
                    BallError::InvalidConfig(format!(
                        "failed to insert lock entry for '{}': {}",
                        pkg.name, e
                    ))
                })?;
        }

        Ok(())
    }
}

fn serialize_source(source: &PackageSource) -> (String, Option<String>) {
    match source {
        PackageSource::GitHub { owner, repo } => {
            ("github".to_string(), Some(format!("{}/{}", owner, repo)))
        }
        PackageSource::BallerRegistry { url } => ("baller_registry".to_string(), Some(url.clone())),
        PackageSource::Chocolatey { feed_url } => {
            ("chocolatey".to_string(), Some(feed_url.clone()))
        }
        PackageSource::System { manager } => ("system".to_string(), Some(manager.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_db_path() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join("baller_test_db");
        let _ = std::fs::create_dir_all(&dir);
        // Include the process id so concurrent/repeated test binaries (common on
        // Windows CI) never collide on the same on-disk file name.
        dir.join(format!("test_{}_{}.db", std::process::id(), n))
    }

    fn make_pkg(name: &str, version: &str) -> Package {
        Package {
            name: name.to_string(),
            version: version.to_string(),
            description: Some("test desc".to_string()),
            author: Some("test author".to_string()),
            repository: Some("https://github.com/test/repo".to_string()),
            architectures: None,
            dependencies: Some(vec!["dep1".to_string(), "dep2 >=1.0".to_string()]),
            sha256: Some("abc123".to_string()),
            hash_algorithm: None,
            download_url: Some("https://example.com/pkg.tar.gz".to_string()),
            source: PackageSource::GitHub {
                owner: "owner".to_string(),
                repo: name.to_string(),
            },
        }
    }

    fn init_db(db_path: &PathBuf) -> DbManager {
        DbManager::init_at_path(db_path).unwrap()
    }

    // NOTE ON TEST TEARDOWN: every test ends with `drop(db);` before
    // `remove_file`. The DbManager owns an open sqlite Connection (holding an OS
    // file handle); on Windows a file cannot be deleted while a handle is open,
    // so dropping the connection first is required to avoid leaking .db files.

    #[test]
    fn test_db_init_creates_schema() {
        let path = test_db_path();
        let db = init_db(&path);
        let count = db.package_count().unwrap();
        assert_eq!(count, 0);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_insert_and_get_package() {
        let path = test_db_path();
        let db = init_db(&path);
        let pkg = make_pkg("test-pkg", "1.0.0");

        db.insert_package(
            &pkg,
            "/install/path",
            Some("/bin/path"),
            Some("/manifest"),
            true,
        )
        .unwrap();

        let retrieved = db.get_package("test-pkg").unwrap();
        assert_eq!(retrieved.name, "test-pkg");
        assert_eq!(retrieved.version, "1.0.0");
        assert_eq!(retrieved.source, "github");
        assert_eq!(retrieved.source_detail.unwrap(), "owner/test-pkg");
        assert_eq!(retrieved.description.unwrap(), "test desc");
        assert_eq!(retrieved.sha256.unwrap(), "abc123");
        assert_eq!(retrieved.install_path, "/install/path");
        assert_eq!(retrieved.bin_path.unwrap(), "/bin/path");
        assert!(!retrieved.frozen);

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_insert_upsert() {
        let path = test_db_path();
        let db = init_db(&path);
        let pkg1 = make_pkg("test-pkg", "1.0.0");
        db.insert_package(&pkg1, "/path1", None, None, true)
            .unwrap();

        let pkg2 = make_pkg("test-pkg", "2.0.0");
        db.insert_package(&pkg2, "/path2", None, None, true)
            .unwrap();

        let retrieved = db.get_package("test-pkg").unwrap();
        assert_eq!(retrieved.version, "2.0.0");
        assert_eq!(retrieved.install_path, "/path2");

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_remove_package() {
        let path = test_db_path();
        let db = init_db(&path);
        let pkg = make_pkg("remove-me", "1.0.0");
        db.insert_package(&pkg, "/path", None, None, true).unwrap();
        assert!(db.package_exists("remove-me").unwrap());

        db.remove_package("remove-me").unwrap();
        assert!(!db.package_exists("remove-me").unwrap());

        let result = db.remove_package("remove-me");
        assert!(result.is_err());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_get_package_not_found() {
        let path = test_db_path();
        let db = init_db(&path);
        let result = db.get_package("nonexistent");
        assert!(result.is_err());
        match result.unwrap_err() {
            BallError::PackageNotFound(name) => assert_eq!(name, "nonexistent"),
            _ => panic!("expected PackageNotFound"),
        }

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_list_packages() {
        let path = test_db_path();
        let db = init_db(&path);

        let pkg1 = make_pkg("alpha", "1.0.0");
        let pkg2 = make_pkg("beta", "2.0.0");
        db.insert_package(&pkg1, "/a", None, None, true).unwrap();
        db.insert_package(&pkg2, "/b", None, None, true).unwrap();

        let pkgs = db.list_packages().unwrap();
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "alpha");
        assert_eq!(pkgs[1].name, "beta");

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_list_packages_empty() {
        let path = test_db_path();
        let db = init_db(&path);
        let pkgs = db.list_packages().unwrap();
        assert!(pkgs.is_empty());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_search_installed() {
        let path = test_db_path();
        let db = init_db(&path);

        let pkg = make_pkg("myapp", "1.0.0");
        db.insert_package(&pkg, "/path", None, None, true).unwrap();

        let results = db.search_installed("myapp").unwrap();
        assert_eq!(results.len(), 1);

        let results = db.search_installed("nonexistent").unwrap();
        assert!(results.is_empty());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_freeze_operations() {
        let path = test_db_path();
        let db = init_db(&path);

        let pkg = make_pkg("freeze-me", "1.0.0");
        db.insert_package(&pkg, "/path", None, None, true).unwrap();

        assert!(!db.is_frozen("freeze-me").unwrap());

        db.set_frozen("freeze-me", true).unwrap();
        assert!(db.is_frozen("freeze-me").unwrap());

        db.set_frozen("freeze-me", false).unwrap();
        assert!(!db.is_frozen("freeze-me").unwrap());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_set_frozen_nonexistent() {
        let path = test_db_path();
        let db = init_db(&path);
        let result = db.set_frozen("nonexistent", true);
        assert!(result.is_err());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_is_frozen_nonexistent() {
        let path = test_db_path();
        let db = init_db(&path);
        assert!(!db.is_frozen("nonexistent").unwrap()); // returns false default

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_package_count() {
        let path = test_db_path();
        let db = init_db(&path);
        assert_eq!(db.package_count().unwrap(), 0);

        let pkg = make_pkg("count-me", "1.0.0");
        db.insert_package(&pkg, "/p", None, None, true).unwrap();
        assert_eq!(db.package_count().unwrap(), 1);

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_list_frozen() {
        let path = test_db_path();
        let db = init_db(&path);

        let pkg1 = make_pkg("frozen-pkg", "1.0.0");
        let pkg2 = make_pkg("thawed-pkg", "2.0.0");
        db.insert_package(&pkg1, "/p1", None, None, true).unwrap();
        db.insert_package(&pkg2, "/p2", None, None, true).unwrap();

        db.set_frozen("frozen-pkg", true).unwrap();

        let frozen = db.list_frozen().unwrap();
        assert_eq!(frozen.len(), 1);
        assert_eq!(frozen[0].name, "frozen-pkg");

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_dependencies() {
        let path = test_db_path();
        let db = init_db(&path);

        let pkg = make_pkg("with-deps", "1.0.0");
        db.insert_package(&pkg, "/p", None, None, true).unwrap();

        let deps = db.get_dependencies("with-deps").unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].0, "dep1");
        assert_eq!(deps[1].0, "dep2");

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_dependencies_empty() {
        let path = test_db_path();
        let db = init_db(&path);

        let pkg = Package::new("no-deps", "1.0.0");
        db.insert_package(&pkg, "/p", None, None, true).unwrap();

        let deps = db.get_dependencies("no-deps").unwrap();
        assert!(deps.is_empty());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_lockfile_operations() {
        let path = test_db_path();
        let db = init_db(&path);

        db.insert_lock_entry("lock-pkg", "1.0.0", "github", Some("sha256hash"))
            .unwrap();

        let entry = db.get_lock_entry("lock-pkg").unwrap();
        assert_eq!(entry.0, "1.0.0");
        assert_eq!(entry.1, "github");
        assert_eq!(entry.2.unwrap(), "sha256hash");

        db.remove_lock_entry("lock-pkg").unwrap();
        let result = db.get_lock_entry("lock-pkg");
        assert!(result.is_err());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_get_all_lock_entries() {
        let path = test_db_path();
        let db = init_db(&path);

        db.insert_lock_entry("a", "1.0", "github", None).unwrap();
        db.insert_lock_entry("b", "2.0", "baller", Some("hash"))
            .unwrap();

        let entries = db.get_all_lock_entries().unwrap();
        assert_eq!(entries.len(), 2);

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_sync_lockfile_from_installed() {
        let path = test_db_path();
        let db = init_db(&path);

        let pkg = make_pkg("sync-pkg", "1.0.0");
        db.insert_package(&pkg, "/p", None, None, true).unwrap();

        db.sync_lockfile_from_installed().unwrap();

        let entry = db.get_lock_entry("sync-pkg").unwrap();
        assert_eq!(entry.0, "1.0.0");

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_insert_and_get_with_bin_path() {
        let path = test_db_path();
        let db = init_db(&path);

        let pkg = make_pkg("bin-pkg", "1.0.0");
        db.insert_package(&pkg, "/install/path", Some("/bin/path"), None, true)
            .unwrap();

        let retrieved = db.get_package("bin-pkg").unwrap();
        assert_eq!(retrieved.install_path, "/install/path");
        assert_eq!(retrieved.bin_path.unwrap(), "/bin/path");

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_remove_package_clears_deps() {
        let path = test_db_path();
        let db = init_db(&path);

        let pkg = make_pkg("dep-clear", "1.0.0");
        db.insert_package(&pkg, "/p", None, None, true).unwrap();

        let deps_before = db.get_dependencies("dep-clear").unwrap();
        assert!(!deps_before.is_empty());

        db.remove_package("dep-clear").unwrap();

        // deps should be gone since cascade delete
        let deps_after = db.get_dependencies("dep-clear").unwrap();
        assert!(deps_after.is_empty());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_insert_and_get_system_source_package() {
        let path = test_db_path();
        let db = init_db(&path);

        let pkg = Package {
            name: "system-pkg".to_string(),
            version: "1.0.0".to_string(),
            description: Some("From system PM".to_string()),
            author: Some("APT".to_string()),
            repository: None,
            architectures: None,
            dependencies: Some(vec!["libc6".to_string()]),
            sha256: None,
            hash_algorithm: None,
            download_url: None,
            source: PackageSource::System {
                manager: "apt".to_string(),
            },
        };

        db.insert_package(&pkg, "/usr/lib", None, None, true)
            .unwrap();

        let retrieved = db.get_package("system-pkg").unwrap();
        assert_eq!(retrieved.name, "system-pkg");
        assert_eq!(retrieved.version, "1.0.0");
        assert_eq!(retrieved.source, "system");
        assert_eq!(retrieved.source_detail, Some("apt".to_string()));
        assert_eq!(retrieved.description.unwrap(), "From system PM");

        drop(db);
        let _ = std::fs::remove_file(&path);
    }
}
