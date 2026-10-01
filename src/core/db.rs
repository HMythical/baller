use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;

use crate::core::package::{Package, PackageSource};
use crate::error::error::BallError;
use crate::platform::common::PlatformManager;
use crate::utils::fs::ensure_dir;

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

#[derive(Debug, Clone, Serialize)]
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
    /// The package's self-declared advisory identity, as JSON.
    ///
    /// Kept on the roster so `baller referee` can re-check a package under the
    /// same identity the install checked it under. Without it an audit would
    /// know strictly less than the install did, which is the one thing an
    /// audit must not do.
    pub advisory: Option<String>,
    #[allow(dead_code)]
    pub dependencies: Vec<String>,
}

impl InstalledPackage {
    /// Rebuild the `Package` this roster row was recorded from.
    ///
    /// Lossy by design: the roster keeps what an install produced, not the
    /// registry document it came from. It keeps enough for Referee — name,
    /// version, source and repository — which is exactly what advisory
    /// identities are built out of.
    pub fn to_package(&self) -> Package {
        Package {
            name: self.name.clone(),
            version: self.version.clone(),
            description: self.description.clone(),
            author: self.author.clone(),
            repository: self.repository.clone(),
            architectures: None,
            dependencies: None,
            sha256: self.sha256.clone(),
            hash_algorithm: None,
            download_url: self.download_url.clone(),
            source: deserialize_source(&self.source, self.source_detail.as_deref()),
            advisory: self
                .advisory
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok()),
            vulnerabilities: Vec::new(),
        }
    }
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
                installed_at TEXT NOT NULL DEFAULT (datetime('now')),
                advisory TEXT
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
            );

            CREATE TABLE IF NOT EXISTS referee_cache (
                ecosystem   TEXT NOT NULL,
                name        TEXT NOT NULL,
                version     TEXT NOT NULL,
                verdict     TEXT NOT NULL,
                risk        REAL,
                advisories  TEXT NOT NULL,
                checked_at  TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (ecosystem, name, version)
            );",
        )
        .map_err(|e| BallError::InvalidConfig(format!("failed to create schema: {}", e)))?;

        Self::migrate_user_installed(&conn)?;

        // Migration: add the advisory column to databases written before
        // Referee existed. Existing rows get NULL, which reads as "declared
        // nothing" — the same answer a package without the section gives.
        let has_advisory: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('installed_packages') WHERE name='advisory'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if has_advisory == 0 {
            let _ = conn.execute(
                "ALTER TABLE installed_packages ADD COLUMN advisory TEXT",
                [],
            );
        }

        Ok(Self { conn })
    }

    /// Adds the `user_installed` column to an `installed_packages` table created
    /// before the column existed. Does nothing when the column is already there, so
    /// it is safe on a fresh database and on every later initialization.
    ///
    /// Returns `Err(InvalidConfig)` when the schema cannot be inspected or the
    /// column cannot be added, with a message naming which of the two failed.
    fn migrate_user_installed(conn: &Connection) -> Result<(), BallError> {
        let has_column: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('installed_packages') WHERE name='user_installed'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| {
                BallError::InvalidConfig(format!(
                    "failed to inspect installed_packages schema for user_installed column: {}",
                    e
                ))
            })?;

        if has_column == 0 {
            // No backfill needed: NOT NULL DEFAULT 1 marks every existing row as user-installed
            conn.execute(
                "ALTER TABLE installed_packages ADD COLUMN user_installed BOOLEAN NOT NULL DEFAULT 1",
                [],
            )
            .map_err(|e| {
                BallError::InvalidConfig(format!(
                    "failed to migrate installed_packages: could not add user_installed column: {}",
                    e
                ))
            })?;
        }

        Ok(())
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
        // A declaration that cannot be re-encoded is dropped rather than
        // failing the install: it is metadata about the package, not the
        // package.
        let advisory = pkg
            .advisory
            .as_ref()
            .and_then(|declaration| serde_json::to_string(declaration).ok());

        self.conn.execute(
            "INSERT INTO installed_packages (name, version, source, source_detail, description, author, repository, download_url, sha256, user_installed, install_path, bin_path, manifest_path, advisory)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
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
                 advisory=excluded.advisory,
                 installed_at=datetime('now')",
            params![
                pkg.name, pkg.version, source, source_detail,
                pkg.description, pkg.author, pkg.repository,
                pkg.download_url, pkg.sha256, user_installed, install_path, bin_path, manifest_path,
                advisory
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
                    download_url, sha256, frozen, user_installed, install_path, bin_path, manifest_path, installed_at, advisory
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
                    advisory: row.get(15)?,
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
                    download_url, sha256, frozen, user_installed, install_path, bin_path, manifest_path, installed_at, advisory
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
                    advisory: row.get(15)?,
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
                    download_url, sha256, frozen, user_installed, install_path, bin_path, manifest_path, installed_at, advisory
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
                    advisory: row.get(15)?,
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

    /// Returns the stored frozen flag, or `false` for a package not on the roster.
    /// Any other lookup failure is an `Err(InvalidConfig)`, never "not frozen".
    pub fn is_frozen(&self, name: &str) -> Result<bool, BallError> {
        match self.conn.query_row(
            "SELECT frozen FROM installed_packages WHERE name = ?1",
            params![name],
            |row| row.get(0),
        ) {
            Ok(frozen) => Ok(frozen),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(BallError::InvalidConfig(format!(
                "failed to read frozen state of package '{}': {}",
                name, e
            ))),
        }
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
                    download_url, sha256, frozen, user_installed, install_path, bin_path, manifest_path, installed_at, advisory
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
                    advisory: row.get(15)?,
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
    /// A Referee verdict recorded earlier for this exact identity and version.
    pub fn referee_cache_get(
        &self,
        ecosystem: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<CachedVerdict>, BallError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT verdict, risk, advisories, checked_at FROM referee_cache
                 WHERE ecosystem = ?1 AND name = ?2 AND version = ?3",
            )
            .map_err(|e| BallError::InvalidConfig(format!("referee cache query error: {}", e)))?;

        let row = stmt.query_row(params![ecosystem, name, version], |row| {
            Ok(CachedVerdict {
                verdict: row.get(0)?,
                risk: row.get(1)?,
                advisories: row.get(2)?,
                checked_at: row.get(3)?,
            })
        });

        match row {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(BallError::InvalidConfig(format!(
                "failed to read the referee cache: {}",
                e
            ))),
        }
    }

    /// A cached verdict that may still be acted on, and how long it has stood.
    ///
    /// `ttl_days` bounds how old a `clean` verdict may be. A `vulnerable` row
    /// is exempt: re-querying one could only confirm the block it already
    /// causes, so the answer is used whatever its age. `None` keeps every row,
    /// which is baller's historical behaviour.
    pub fn referee_cache_get_fresh(
        &self,
        ecosystem: &str,
        name: &str,
        version: &str,
        ttl_days: Option<u32>,
    ) -> Result<Option<CachedVerdict>, BallError> {
        // Strictly newer than the cutoff: a row written this second is not
        // fresh under a TTL of `0`, which must re-ask every time.
        let cutoff: Option<String> = ttl_days.map(|days| format!("-{} days", days));
        let mut stmt = self
            .conn
            .prepare(
                "SELECT verdict, risk, advisories, checked_at FROM referee_cache
                 WHERE ecosystem = ?1 AND name = ?2 AND version = ?3
                   AND (verdict = 'vulnerable' OR ?4 IS NULL
                        OR checked_at > datetime('now', ?4))",
            )
            .map_err(|e| BallError::InvalidConfig(format!("referee cache query error: {}", e)))?;

        let row = stmt.query_row(params![ecosystem, name, version, cutoff], |row| {
            Ok(CachedVerdict {
                verdict: row.get(0)?,
                risk: row.get(1)?,
                advisories: row.get(2)?,
                checked_at: row.get(3)?,
            })
        });

        match row {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(BallError::InvalidConfig(format!(
                "failed to read the referee cache: {}",
                e
            ))),
        }
    }

    /// Whole days since `checked_at`, as SQLite counts them.
    ///
    /// `checked_at` is SQLite's own `datetime('now')` text, so the arithmetic
    /// stays in SQLite rather than pulling in a date crate.
    pub fn referee_age_days(&self, checked_at: &str) -> Option<i64> {
        self.conn
            .query_row(
                "SELECT CAST(julianday('now') - julianday(?1) AS INTEGER)",
                params![checked_at],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
    }

    /// Age a cached verdict by a SQLite datetime modifier, e.g. `-40 days`.
    ///
    /// Lets the Referee tests prove a row has aged past the TTL without
    /// sleeping or reaching for a date crate.
    #[cfg(test)]
    pub fn referee_cache_age_for_test(
        &self,
        ecosystem: &str,
        name: &str,
        version: &str,
        modifier: &str,
    ) -> Result<usize, BallError> {
        self.conn
            .execute(
                "UPDATE referee_cache SET checked_at = datetime('now', ?4)
                 WHERE ecosystem = ?1 AND name = ?2 AND version = ?3",
                params![ecosystem, name, version, modifier],
            )
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to age the referee cache: {}", e))
            })
    }

    /// Record a verdict, replacing any earlier one for the same key.
    #[allow(clippy::too_many_arguments)]
    pub fn referee_cache_put(
        &self,
        ecosystem: &str,
        name: &str,
        version: &str,
        verdict: &str,
        risk: Option<f32>,
        advisories: &str,
    ) -> Result<(), BallError> {
        self.conn
            .execute(
                "INSERT INTO referee_cache (ecosystem, name, version, verdict, risk, advisories, checked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
                 ON CONFLICT(ecosystem, name, version) DO UPDATE SET
                     verdict=excluded.verdict,
                     risk=excluded.risk,
                     advisories=excluded.advisories,
                     checked_at=excluded.checked_at",
                params![
                    ecosystem,
                    name,
                    version,
                    verdict,
                    risk.map(|risk| risk as f64),
                    advisories
                ],
            )
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to write the referee cache: {}", e))
            })?;
        Ok(())
    }

    /// Drop every cached verdict, for `referee --refresh`.
    pub fn referee_cache_clear(&self) -> Result<usize, BallError> {
        self.conn
            .execute("DELETE FROM referee_cache", [])
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to clear the referee cache: {}", e))
            })
    }

    /// How many verdicts are cached.
    pub fn referee_cache_count(&self) -> Result<i64, BallError> {
        self.conn
            .query_row("SELECT COUNT(*) FROM referee_cache", [], |row| row.get(0))
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to count the referee cache: {}", e))
            })
    }

    /// Cached verdicts per ecosystem, with the newest `checked_at` of each,
    /// for `referee cache --status`.
    pub fn referee_cache_stats(&self) -> Result<Vec<RefereeCacheStats>, BallError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ecosystem, COUNT(*), MAX(checked_at) FROM referee_cache
                 GROUP BY ecosystem ORDER BY ecosystem",
            )
            .map_err(|e| BallError::InvalidConfig(format!("referee cache query error: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(RefereeCacheStats {
                    ecosystem: row.get(0)?,
                    count: row.get(1)?,
                    newest: row.get(2)?,
                })
            })
            .map_err(|e| {
                BallError::InvalidConfig(format!("failed to read the referee cache: {}", e))
            })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
            BallError::InvalidConfig(format!("failed to read the referee cache: {}", e))
        })
    }

    /// Drop verdicts computed more than `days` days ago, for
    /// `referee cache --prune`.
    ///
    /// `vulnerable` rows are kept unless `include_vulnerable` is set. The TTL
    /// in [`Self::referee_cache_get_fresh`] honours a block no matter how old
    /// it is, and pruning must agree: deleting a vulnerable verdict turns a
    /// known vulnerability back into "no data", which an audit then reports as
    /// `unknown` or `unverified`. How many vulnerable rows were kept is
    /// returned so the command can say what it declined to delete — a silent
    /// non-deletion is as misleading as a silent deletion.
    pub fn referee_cache_prune(
        &self,
        days: u32,
        include_vulnerable: bool,
    ) -> Result<RefereeCachePrune, BallError> {
        let to_err = |e: rusqlite::Error| {
            BallError::InvalidConfig(format!("failed to prune the referee cache: {}", e))
        };
        let cutoff = format!("-{} days", days);

        let old_vulnerable: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM referee_cache
                 WHERE verdict = 'vulnerable' AND checked_at < datetime('now', ?1)",
                params![cutoff],
                |row| row.get(0),
            )
            .map_err(to_err)?;

        let sql = if include_vulnerable {
            "DELETE FROM referee_cache WHERE checked_at < datetime('now', ?1)"
        } else {
            "DELETE FROM referee_cache
             WHERE checked_at < datetime('now', ?1) AND verdict <> 'vulnerable'"
        };
        let removed = self.conn.execute(sql, params![cutoff]).map_err(to_err)?;

        let old_vulnerable = old_vulnerable as usize;
        Ok(if include_vulnerable {
            RefereeCachePrune {
                removed,
                removed_vulnerable: old_vulnerable,
                kept_vulnerable: 0,
            }
        } else {
            RefereeCachePrune {
                removed,
                removed_vulnerable: 0,
                kept_vulnerable: old_vulnerable,
            }
        })
    }

    /// How many `clean` verdicts are older than `ttl_days` and will therefore
    /// be re-queried on the next install. `None` is always `0`: with no TTL
    /// nothing ages out.
    pub fn referee_cache_stale_count(&self, ttl_days: Option<u32>) -> Result<i64, BallError> {
        let Some(days) = ttl_days else {
            return Ok(0);
        };
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM referee_cache
                 WHERE verdict = 'clean' AND checked_at <= datetime('now', ?1)",
                params![format!("-{} days", days)],
                |row| row.get(0),
            )
            .map_err(|e| BallError::InvalidConfig(format!("failed to count stale verdicts: {}", e)))
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

/// The inverse of [`serialize_source`].
///
/// An unrecognised `source` column — written by another build, or hand-edited —
/// falls back to a GitHub source with no owner, which produces no advisory
/// identity at all. That reads as `Unknown`, which is the truthful answer for a
/// row baller cannot interpret.
pub fn deserialize_source(source: &str, detail: Option<&str>) -> PackageSource {
    let detail = detail.unwrap_or("").trim();

    match source {
        "github" => {
            let (owner, repo) = detail.split_once('/').unwrap_or(("", detail));
            PackageSource::GitHub {
                owner: owner.to_string(),
                repo: repo.to_string(),
            }
        }
        "baller_registry" => PackageSource::BallerRegistry {
            url: detail.to_string(),
        },
        "chocolatey" => PackageSource::Chocolatey {
            feed_url: detail.to_string(),
        },
        "system" => PackageSource::System {
            manager: detail.to_string(),
        },
        "cargo" => PackageSource::Cargo {
            crate_name: detail.to_string(),
        },
        other => {
            tracing::debug!("unrecognised roster source '{}'", other);
            PackageSource::GitHub {
                owner: String::new(),
                repo: String::new(),
            }
        }
    }
}

/// A verdict recorded by an earlier Referee run.
///
/// Cached rows are keyed by the exact `(ecosystem, name, version)` they were
/// computed from: a `Clean` verdict says nothing about any other version, and
/// reusing it for one would be the whole point of the cache getting it wrong.
#[derive(Debug, Clone)]
pub struct CachedVerdict {
    pub verdict: String,
    pub risk: Option<f64>,
    /// The matched advisories, as the JSON `referee_cache.advisories` holds
    pub advisories: String,
    pub checked_at: String,
}

/// One ecosystem's share of the verdict cache.
#[derive(Debug, Clone, PartialEq)]
pub struct RefereeCacheStats {
    pub ecosystem: String,
    pub count: i64,
    /// The most recent `checked_at` in this ecosystem
    pub newest: Option<String>,
}

/// What `referee cache --prune` did, and what it declined to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefereeCachePrune {
    /// Rows deleted, of every verdict
    pub removed: usize,
    /// Of `removed`, how many were `vulnerable` (only with the opt-in)
    pub removed_vulnerable: usize,
    /// `vulnerable` rows past the cutoff left in place (only without it)
    pub kept_vulnerable: usize,
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
        PackageSource::Cargo { crate_name } => ("cargo".to_string(), Some(crate_name.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::registry::RegistrySource;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn test_registry_source_db_names_match_stored_values() {
        let cases = [
            (
                PackageSource::GitHub {
                    owner: "o".to_string(),
                    repo: "r".to_string(),
                },
                RegistrySource::GitHub,
            ),
            (
                PackageSource::BallerRegistry {
                    url: "u".to_string(),
                },
                RegistrySource::BallerRegistry,
            ),
            (
                PackageSource::Chocolatey {
                    feed_url: "f".to_string(),
                },
                RegistrySource::Chocolatey,
            ),
            (
                PackageSource::System {
                    manager: "apt".to_string(),
                },
                RegistrySource::System,
            ),
            (
                PackageSource::Cargo {
                    crate_name: "ripgrep".to_string(),
                },
                RegistrySource::Cargo,
            ),
        ];

        for (package_source, registry_source) in &cases {
            let (stored, _) = serialize_source(package_source);
            assert_eq!(
                stored,
                registry_source.db_name(),
                "roster --source filter must match what the DB stores"
            );
        }
    }

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
            advisory: None,
            vulnerabilities: Vec::new(),
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
    fn test_advisory_declaration_survives_the_roster() {
        use crate::core::package::AdvisoryDeclaration;

        let path = test_db_path();
        let db = init_db(&path);

        let mut pkg = make_pkg("declares", "1.0.0");
        pkg.advisory = Some(AdvisoryDeclaration {
            ecosystem: Some("crates.io".to_string()),
            name: Some("declares".to_string()),
            aliases: vec!["CVE-2026-1".to_string()],
        });
        db.insert_package(&pkg, "/install", None, None, true)
            .unwrap();

        let row = db.get_package("declares").unwrap();
        assert!(row.advisory.is_some());

        // The audit must be able to check a package under the same identity
        // the install checked it under.
        let restored = row.to_package();
        assert_eq!(restored.advisory, pkg.advisory);
        assert_eq!(
            restored.advisory_identities(),
            vec![("crates.io".to_string(), "declares".to_string())]
        );
        assert_eq!(restored.declared_aliases(), ["CVE-2026-1".to_string()]);

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_a_package_without_a_declaration_stores_null() {
        let path = test_db_path();
        let db = init_db(&path);
        db.insert_package(&make_pkg("plain", "1.0.0"), "/install", None, None, true)
            .unwrap();

        let row = db.get_package("plain").unwrap();
        assert!(row.advisory.is_none());
        assert!(row.to_package().advisory.is_none());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_a_pre_referee_database_gains_the_advisory_column() {
        let path = test_db_path();
        {
            // A database written before Referee existed: no advisory column.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE installed_packages (
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
                INSERT INTO installed_packages (name, version, install_path)
                VALUES ('legacy', '0.9.0', '/old/path');",
            )
            .unwrap();
        }

        let db = init_db(&path);
        let row = db.get_package("legacy").unwrap();
        assert_eq!(row.version, "0.9.0");
        assert!(row.advisory.is_none());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_referee_cache_round_trip() {
        let path = test_db_path();
        let db = init_db(&path);

        assert!(db
            .referee_cache_get("crates.io", "serde", "1.0.0")
            .unwrap()
            .is_none());
        assert_eq!(db.referee_cache_count().unwrap(), 0);

        db.referee_cache_put(
            "crates.io",
            "serde",
            "1.0.0",
            "vulnerable",
            Some(3.75),
            "[{\"id\":\"GHSA-a\",\"aliases\":[],\"cvss\":7.5,\"summary\":null}]",
        )
        .unwrap();

        let cached = db
            .referee_cache_get("crates.io", "serde", "1.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(cached.verdict, "vulnerable");
        assert!((cached.risk.unwrap() - 3.75).abs() < 1e-6);
        assert!(cached.advisories.contains("GHSA-a"));
        assert!(!cached.checked_at.is_empty());

        // A different version is a different key.
        assert!(db
            .referee_cache_get("crates.io", "serde", "1.0.1")
            .unwrap()
            .is_none());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_referee_cache_put_overwrites_the_same_key() {
        let path = test_db_path();
        let db = init_db(&path);

        db.referee_cache_put("crates.io", "serde", "1.0.0", "clean", None, "[]")
            .unwrap();
        db.referee_cache_put("crates.io", "serde", "1.0.0", "vulnerable", Some(5.0), "[]")
            .unwrap();

        assert_eq!(db.referee_cache_count().unwrap(), 1);
        let cached = db
            .referee_cache_get("crates.io", "serde", "1.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(cached.verdict, "vulnerable");

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_referee_cache_clear_empties_it() {
        let path = test_db_path();
        let db = init_db(&path);

        db.referee_cache_put("crates.io", "a", "1.0.0", "clean", None, "[]")
            .unwrap();
        db.referee_cache_put("NuGet", "b", "2.0.0", "clean", None, "[]")
            .unwrap();
        assert_eq!(db.referee_cache_count().unwrap(), 2);

        assert_eq!(db.referee_cache_clear().unwrap(), 2);
        assert_eq!(db.referee_cache_count().unwrap(), 0);

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    /// Backdate one cached verdict, as if it had been computed `days` ago.
    fn age_verdict(db: &DbManager, name: &str, days: u32) {
        db.conn
            .execute(
                "UPDATE referee_cache SET checked_at = datetime('now', ?1) WHERE name = ?2",
                params![format!("-{} days", days), name],
            )
            .unwrap();
    }

    #[test]
    fn test_referee_cache_stats_group_per_ecosystem() {
        let path = test_db_path();
        let db = init_db(&path);

        assert!(db.referee_cache_stats().unwrap().is_empty());

        db.referee_cache_put("crates.io", "a", "1.0.0", "clean", None, "[]")
            .unwrap();
        db.referee_cache_put("crates.io", "b", "1.0.0", "clean", None, "[]")
            .unwrap();
        db.referee_cache_put("NuGet", "c", "2.0.0", "vulnerable", Some(4.5), "[]")
            .unwrap();
        age_verdict(&db, "a", 10);

        let stats = db.referee_cache_stats().unwrap();
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].ecosystem, "NuGet");
        assert_eq!(stats[0].count, 1);
        assert_eq!(stats[1].ecosystem, "crates.io");
        assert_eq!(stats[1].count, 2);

        // The newest crates.io row is `b`, not the backdated `a`.
        let b = db
            .referee_cache_get("crates.io", "b", "1.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(stats[1].newest.as_deref(), Some(b.checked_at.as_str()));

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_referee_cache_prune_keeps_vulnerable_rows_by_default() {
        let path = test_db_path();
        let db = init_db(&path);

        db.referee_cache_put("crates.io", "old-clean", "1.0.0", "clean", None, "[]")
            .unwrap();
        db.referee_cache_put(
            "crates.io",
            "old-bad",
            "1.0.0",
            "vulnerable",
            Some(4.5),
            "[]",
        )
        .unwrap();
        db.referee_cache_put(
            "crates.io",
            "new-bad",
            "1.0.0",
            "vulnerable",
            Some(4.5),
            "[]",
        )
        .unwrap();
        age_verdict(&db, "old-clean", 40);
        age_verdict(&db, "old-bad", 40);

        let outcome = db.referee_cache_prune(30, false).unwrap();
        assert_eq!(
            outcome,
            RefereeCachePrune {
                removed: 1,
                removed_vulnerable: 0,
                kept_vulnerable: 1,
            }
        );
        assert!(db
            .referee_cache_get("crates.io", "old-bad", "1.0.0")
            .unwrap()
            .is_some());

        // A second run still reports the vulnerable row it is keeping.
        assert_eq!(
            db.referee_cache_prune(30, false).unwrap().kept_vulnerable,
            1
        );

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_referee_cache_prune_opt_in_removes_vulnerable_rows() {
        let path = test_db_path();
        let db = init_db(&path);

        db.referee_cache_put("crates.io", "old-clean", "1.0.0", "clean", None, "[]")
            .unwrap();
        db.referee_cache_put(
            "crates.io",
            "old-bad",
            "1.0.0",
            "vulnerable",
            Some(4.5),
            "[]",
        )
        .unwrap();
        db.referee_cache_put(
            "crates.io",
            "new-bad",
            "1.0.0",
            "vulnerable",
            Some(4.5),
            "[]",
        )
        .unwrap();
        age_verdict(&db, "old-clean", 40);
        age_verdict(&db, "old-bad", 40);

        let outcome = db.referee_cache_prune(30, true).unwrap();
        assert_eq!(
            outcome,
            RefereeCachePrune {
                removed: 2,
                removed_vulnerable: 1,
                kept_vulnerable: 0,
            }
        );
        // Only rows past the cutoff go, whatever their verdict.
        assert!(db
            .referee_cache_get("crates.io", "new-bad", "1.0.0")
            .unwrap()
            .is_some());

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_referee_cache_prune_drops_only_stale_rows() {
        let path = test_db_path();
        let db = init_db(&path);

        db.referee_cache_put("crates.io", "old", "1.0.0", "clean", None, "[]")
            .unwrap();
        db.referee_cache_put("crates.io", "older", "1.0.0", "clean", None, "[]")
            .unwrap();
        db.referee_cache_put("crates.io", "fresh", "1.0.0", "clean", None, "[]")
            .unwrap();
        age_verdict(&db, "old", 10);
        age_verdict(&db, "older", 40);

        assert_eq!(db.referee_cache_prune(30, false).unwrap().removed, 1);
        assert!(db
            .referee_cache_get("crates.io", "older", "1.0.0")
            .unwrap()
            .is_none());
        assert_eq!(db.referee_cache_count().unwrap(), 2);

        assert_eq!(db.referee_cache_prune(7, false).unwrap().removed, 1);
        assert!(db
            .referee_cache_get("crates.io", "fresh", "1.0.0")
            .unwrap()
            .is_some());
        assert_eq!(db.referee_cache_count().unwrap(), 1);

        assert_eq!(db.referee_cache_prune(7, false).unwrap().removed, 0);

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_deserialize_source_round_trips_every_variant() {
        let cases = [
            PackageSource::GitHub {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
            },
            PackageSource::BallerRegistry {
                url: "https://registry.test/api".to_string(),
            },
            PackageSource::Chocolatey {
                feed_url: "https://feed.test/api/v2".to_string(),
            },
            PackageSource::System {
                manager: "apt".to_string(),
            },
            PackageSource::Cargo {
                crate_name: "ripgrep".to_string(),
            },
        ];

        for source in cases {
            let (name, detail) = serialize_source(&source);
            assert_eq!(deserialize_source(&name, detail.as_deref()), source);
        }
    }

    #[test]
    fn test_deserialize_source_of_an_unknown_name_is_inert() {
        let source = deserialize_source("quantum", Some("whatever"));
        assert_eq!(
            source,
            PackageSource::GitHub {
                owner: String::new(),
                repo: String::new()
            }
        );
    }

    #[test]
    fn test_db_init_creates_schema() {
        let path = test_db_path();
        let db = init_db(&path);
        let count = db.package_count().unwrap();
        assert_eq!(count, 0);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    /// A database written before `installed_packages` had a `user_installed` column
    fn legacy_db(path: &PathBuf) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE installed_packages (
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
                install_path TEXT NOT NULL,
                bin_path TEXT,
                manifest_path TEXT,
                installed_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            INSERT INTO installed_packages (name, version, install_path)
                VALUES ('legacy-pkg', '1.0.0', '/p');",
        )
        .unwrap();
        conn
    }

    fn user_installed_columns(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('installed_packages') WHERE name='user_installed'",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn test_init_fails_when_user_installed_migration_fails() {
        let path = test_db_path();
        let conn = legacy_db(&path);
        // A view passes CREATE TABLE IF NOT EXISTS and the schema probe, but
        // cannot be altered, so only the migration step can fail
        conn.execute_batch(
            "ALTER TABLE installed_packages RENAME TO legacy_packages;
             CREATE VIEW installed_packages AS SELECT * FROM legacy_packages;",
        )
        .unwrap();
        drop(conn);

        let err = DbManager::init_at_path(&path)
            .err()
            .expect("init must fail on a database it could not migrate")
            .to_string();
        assert!(
            err.contains("user_installed"),
            "error should name the migration step: {}",
            err
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_migrate_user_installed_adds_column_to_legacy_db() {
        let path = test_db_path();
        let conn = legacy_db(&path);
        assert_eq!(user_installed_columns(&conn), 0);

        DbManager::migrate_user_installed(&conn).unwrap();
        assert_eq!(user_installed_columns(&conn), 1);

        let user_installed: bool = conn
            .query_row(
                "SELECT user_installed FROM installed_packages WHERE name = 'legacy-pkg'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(user_installed, "existing rows default to user-installed");

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_migrate_user_installed_is_idempotent() {
        let path = test_db_path();
        let conn = legacy_db(&path);

        DbManager::migrate_user_installed(&conn).unwrap();
        // The second call must see the column and skip the ALTER, not fail
        // with "duplicate column name"
        DbManager::migrate_user_installed(&conn).unwrap();
        assert_eq!(user_installed_columns(&conn), 1);

        drop(conn);
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
    fn test_is_frozen_propagates_query_errors() {
        let path = test_db_path();
        let db = init_db(&path);
        // A failed lookup is not the same as "not on the roster"
        db.conn
            .execute_batch("DROP TABLE installed_packages;")
            .unwrap();

        let err = db
            .is_frozen("some-pkg")
            .expect_err("a failed lookup must not read as not-frozen")
            .to_string();
        assert!(
            err.contains("some-pkg"),
            "error should name the package: {}",
            err
        );

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
            advisory: None,
            vulnerabilities: Vec::new(),
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

    fn put_verdict(db: &DbManager, name: &str, verdict: &str) {
        let risk = (verdict == "vulnerable").then_some(4.9);
        db.referee_cache_put("crates.io", name, "1.0.0", verdict, risk, "[]")
            .unwrap();
    }

    #[test]
    fn test_referee_cache_get_fresh_without_a_ttl_keeps_every_row() {
        let path = test_db_path();
        let db = init_db(&path);
        put_verdict(&db, "old", "clean");
        db.referee_cache_age_for_test("crates.io", "old", "1.0.0", "-400 days")
            .unwrap();

        assert!(db
            .referee_cache_get_fresh("crates.io", "old", "1.0.0", None)
            .unwrap()
            .is_some());
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_referee_cache_get_fresh_excludes_only_a_stale_clean_row() {
        let path = test_db_path();
        let db = init_db(&path);
        put_verdict(&db, "fresh", "clean");
        put_verdict(&db, "stale", "clean");
        put_verdict(&db, "flagged", "vulnerable");
        db.referee_cache_age_for_test("crates.io", "fresh", "1.0.0", "-1 days")
            .unwrap();
        db.referee_cache_age_for_test("crates.io", "stale", "1.0.0", "-40 days")
            .unwrap();
        db.referee_cache_age_for_test("crates.io", "flagged", "1.0.0", "-400 days")
            .unwrap();

        let get = |name: &str| {
            db.referee_cache_get_fresh("crates.io", name, "1.0.0", Some(7))
                .unwrap()
        };
        assert!(get("fresh").is_some());
        assert!(get("stale").is_none());
        let flagged = get("flagged").expect("a vulnerable row is never aged out");
        assert_eq!(flagged.verdict, "vulnerable");
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_referee_cache_stale_count_counts_only_stale_clean_rows() {
        let path = test_db_path();
        let db = init_db(&path);
        put_verdict(&db, "fresh", "clean");
        put_verdict(&db, "stale", "clean");
        put_verdict(&db, "flagged", "vulnerable");
        db.referee_cache_age_for_test("crates.io", "stale", "1.0.0", "-40 days")
            .unwrap();
        db.referee_cache_age_for_test("crates.io", "flagged", "1.0.0", "-40 days")
            .unwrap();

        assert_eq!(db.referee_cache_stale_count(Some(7)).unwrap(), 1);
        assert_eq!(db.referee_cache_stale_count(None).unwrap(), 0);
        assert_eq!(db.referee_cache_stale_count(Some(0)).unwrap(), 2);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_referee_age_days_counts_whole_days() {
        let path = test_db_path();
        let db = init_db(&path);
        put_verdict(&db, "old", "clean");
        db.referee_cache_age_for_test("crates.io", "old", "1.0.0", "-40 days")
            .unwrap();
        let row = db
            .referee_cache_get("crates.io", "old", "1.0.0")
            .unwrap()
            .unwrap();

        assert_eq!(db.referee_age_days(&row.checked_at), Some(40));
        assert_eq!(db.referee_age_days("not a date"), None);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }
}
