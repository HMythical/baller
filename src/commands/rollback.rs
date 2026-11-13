//! Undoing a partly applied install (#59).
//!
//! `draft`, `substitute` and `update` apply a plan as a sequence of separate,
//! persisted steps — extract, link, record — with no transaction around them.
//! A [`Session`] notes each step as it is taken, so a failure partway through
//! can unwind everything the run did, newest first, before the original error
//! is returned.
//!
//! A package the run found already installed is restored, never removed: a
//! `draft --force` reinstall, a dependency `substitute` re-fetched and the root
//! of an `update` were all working before the run touched them. Its roster row
//! and dependency rows go back as they were and its link points at its old
//! binary again; only a package new to this run is unlinked, taken off the
//! roster and purged from the cache.
//!
//! Rollback is best-effort: a step that cannot be undone is logged at
//! `--verbose` and the unwind carries on, so a cleanup failure never masks the
//! error that caused it. What baller did not do itself — a system package
//! manager's or `cargo install`'s work, or whatever a hook ran — is not undone.

use std::path::Path;

use colored::Colorize;

use crate::context::AppContext;
use crate::core::db::{DbManager, InstalledPackage};
use crate::core::downloader::{DownloadedPackage, Downloader};
use crate::core::package::{Package, PackageSource};
use crate::error::error::BallError;
use crate::platform::common::PlatformManager;

#[cfg(target_os = "linux")]
use crate::platform::linux::LinuxManager as ActiveManager;

#[cfg(target_os = "windows")]
use crate::platform::windows::WindowsManager as ActiveManager;

/// The packages one run has changed, and how far it got with each.
#[derive(Default)]
pub(crate) struct Session {
    entries: Vec<Entry>,
}

/// Everything the run has done to one package so far.
struct Entry {
    package: Package,
    /// Its roster state before this run, if it was already installed.
    previous: Option<Previous>,
    downloaded: Option<DownloadedPackage>,
    linked: bool,
    recorded: bool,
}

/// A roster row as it stood before the run overwrote it.
struct Previous {
    row: InstalledPackage,
    /// `(name, version)` rows from `package_dependencies`, which an insert
    /// rewrites along with the package row.
    dependencies: Vec<(String, String)>,
}

impl Session {
    /// Start tracking `pkg`, before the first step that changes anything for it.
    ///
    /// Snapshots its roster row so rollback can put back a package that was
    /// already installed. A roster that cannot be read is an error: without
    /// the snapshot, rollback could not tell a reinstall from a fresh install
    /// and would remove a package the user already had.
    pub(crate) fn begin(&mut self, db: &DbManager, pkg: &Package) -> Result<(), BallError> {
        let previous = match db.get_package(&pkg.name) {
            Ok(row) => Some(Previous {
                dependencies: db.get_dependencies(&pkg.name)?,
                row,
            }),
            Err(BallError::PackageNotFound(_)) => None,
            Err(e) => return Err(e),
        };

        self.entries.push(Entry {
            package: pkg.clone(),
            previous,
            downloaded: None,
            linked: false,
            recorded: false,
        });
        Ok(())
    }

    /// The current package's archive was fetched and extracted.
    pub(crate) fn downloaded(&mut self, downloaded: &DownloadedPackage) {
        if let Some(entry) = self.entries.last_mut() {
            entry.downloaded = Some(downloaded.clone());
        }
    }

    /// The current package's binary was linked into the install directory.
    pub(crate) fn linked(&mut self) {
        if let Some(entry) = self.entries.last_mut() {
            entry.linked = true;
        }
    }

    /// The current package was written to the roster.
    pub(crate) fn recorded(&mut self) {
        if let Some(entry) = self.entries.last_mut() {
            entry.recorded = true;
        }
    }

    /// The packages this run put on the roster, in install order.
    pub(crate) fn installed(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.recorded)
            .map(|entry| entry.package.name.clone())
            .collect()
    }

    /// Undo every step this run took, newest package first.
    pub(crate) fn rollback(&self, ctx: &AppContext) {
        self.unwind(&ctx.db, &ctx.downloader);
    }

    fn unwind(&self, db: &DbManager, downloader: &Downloader) {
        let touched: Vec<&Entry> = self
            .entries
            .iter()
            .rev()
            .filter(|entry| entry.downloaded.is_some() || entry.linked || entry.recorded)
            .collect();

        if touched.is_empty() {
            return;
        }

        for entry in &touched {
            entry.undo(db, downloader);
        }

        tracing::warn!(
            "{} undid this run's changes to {}",
            "Rolled back".yellow(),
            touched
                .iter()
                .rev()
                .map(|entry| entry.package.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

impl Entry {
    fn undo(&self, db: &DbManager, downloader: &Downloader) {
        let name = &self.package.name;
        tracing::debug!(
            "rolling back {} v{} ({})",
            name,
            self.package.version,
            if self.previous.is_some() {
                "restoring the previous install"
            } else {
                "removing it"
            }
        );

        // The link first: it is what the user runs, and the binary it is put
        // back to lives in the previous install's tree, which this run never
        // deletes.
        if self.linked {
            let previous_bin = self
                .previous
                .as_ref()
                .and_then(|previous| previous.row.bin_path.as_deref());
            let result = match previous_bin {
                Some(bin) => ActiveManager::create_symlink(Path::new(bin), name),
                None => ActiveManager::remove_symlink(name),
            };
            if let Err(e) = result {
                tracing::debug!("{}: could not restore its link: {}", name, e);
            }
        }

        if self.recorded {
            let result = match &self.previous {
                Some(previous) => previous.restore(db),
                None => match db.remove_package(name) {
                    Err(BallError::PackageNotFound(_)) => Ok(()),
                    other => other,
                },
            };
            if let Err(e) = result {
                tracing::debug!("{}: could not restore its roster entry: {}", name, e);
            }

            if self.previous.is_none() {
                if let Some(tool) = native_installer(&self.package.source) {
                    tracing::warn!(
                        "{} {} was installed by {} and stays installed; baller only removed its roster entry",
                        "Note".yellow(),
                        name.cyan(),
                        tool.cyan()
                    );
                }
            }
        }

        if let Some(downloaded) = &self.downloaded {
            // A same-version reinstall extracted over the very tree the
            // restored row points at, so that tree is the install and stays.
            let in_use = self.previous.as_ref().is_some_and(|previous| {
                Path::new(&previous.row.install_path) == downloaded.extract_dir
            });
            if !in_use {
                downloader.purge_download(&self.package, downloaded);
            }
        }
    }
}

impl Previous {
    /// Write the roster row and its dependency rows back as they were.
    ///
    /// `installed_at` is the one column that does not survive: the upsert
    /// stamps it with the time of the restore.
    fn restore(&self, db: &DbManager) -> Result<(), BallError> {
        let mut pkg = self.row.to_package();
        // `insert_package` rewrites the dependency rows from this list, in the
        // `name>=version` form it parses.
        pkg.dependencies = Some(
            self.dependencies
                .iter()
                .map(|(dep, version)| format!("{}>={}", dep, version))
                .collect(),
        );
        db.insert_package(
            &pkg,
            &self.row.install_path,
            self.row.bin_path.as_deref(),
            self.row.manifest_path.as_deref(),
            self.row.user_installed,
        )
    }
}

/// The tool that installed a package outside baller's cache, if any
fn native_installer(source: &PackageSource) -> Option<String> {
    match source {
        PackageSource::System { manager } => Some(manager.clone()),
        PackageSource::Cargo { .. } => Some("cargo".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpClient;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A directory no other test shares, with a database and a cache in it
    fn scratch(tag: &str) -> (PathBuf, DbManager, Downloader) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir()
            .join("baller_rollback_tests")
            .join(format!(
                "{}_{}_{}",
                std::process::id(),
                tag,
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("cache")).unwrap();
        let db = DbManager::init_at_path(&dir.join("baller.db")).unwrap();
        let downloader = Downloader::new(dir.join("cache"), HttpClient::new().unwrap());
        (dir, db, downloader)
    }

    fn package(name: &str, version: &str, deps: &[&str]) -> Package {
        let mut pkg = Package::new(name, version);
        pkg.download_url = Some(format!("https://example.test/{}-{}.tar.gz", name, version));
        pkg.dependencies = Some(deps.iter().map(|dep| dep.to_string()).collect());
        pkg
    }

    /// Lay down `pkg`'s cached archive and extract dir, as a download would
    fn fake_download(dir: &Path, pkg: &Package) -> DownloadedPackage {
        let cache = dir.join("cache");
        let extract_dir = cache.join(format!("{}-{}", pkg.name, pkg.version));
        fs::create_dir_all(&extract_dir).unwrap();
        fs::write(extract_dir.join(&pkg.name), "#!/bin/sh\n").unwrap();
        let archive_path = cache.join(crate::utils::fs::sanitize_filename(
            pkg.download_url.as_deref().unwrap(),
        ));
        fs::write(&archive_path, "archive").unwrap();
        DownloadedPackage {
            archive_path,
            binary_path: Some(extract_dir.join(&pkg.name)),
            extract_dir,
        }
    }

    fn record(db: &DbManager, pkg: &Package, downloaded: &DownloadedPackage, root: bool) {
        db.insert_package(
            pkg,
            &downloaded.extract_dir.to_string_lossy(),
            Some(&downloaded.binary_path.as_ref().unwrap().to_string_lossy()),
            None,
            root,
        )
        .unwrap();
    }

    #[test]
    fn test_a_fresh_package_is_removed_and_purged() {
        let (dir, db, downloader) = scratch("fresh");
        let pkg = package("fresh", "1.0.0", &[]);

        let mut session = Session::default();
        session.begin(&db, &pkg).unwrap();
        let downloaded = fake_download(&dir, &pkg);
        session.downloaded(&downloaded);
        record(&db, &pkg, &downloaded, true);
        session.recorded();

        session.unwind(&db, &downloader);

        assert!(matches!(
            db.get_package("fresh"),
            Err(BallError::PackageNotFound(_))
        ));
        assert!(!downloaded.extract_dir.exists());
        assert!(!downloaded.archive_path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_a_package_that_was_already_installed_is_restored_not_removed() {
        let (dir, db, downloader) = scratch("restore");
        let old = package("kept", "1.0.0", &["libold>=1.0"]);
        let old_download = fake_download(&dir, &old);
        record(&db, &old, &old_download, true);

        let new = package("kept", "2.0.0", &["libnew"]);
        let mut session = Session::default();
        session.begin(&db, &new).unwrap();
        let new_download = fake_download(&dir, &new);
        session.downloaded(&new_download);
        record(&db, &new, &new_download, false);
        session.recorded();

        session.unwind(&db, &downloader);

        let row = db.get_package("kept").unwrap();
        assert_eq!(row.version, "1.0.0");
        assert!(row.user_installed);
        assert_eq!(row.install_path, old_download.extract_dir.to_string_lossy());
        assert_eq!(
            db.get_dependencies("kept").unwrap(),
            vec![("libold".to_string(), "1.0".to_string())]
        );
        // The old tree is the install again; only the new one is purged.
        assert!(old_download.extract_dir.exists());
        assert!(!new_download.extract_dir.exists());
        assert!(!new_download.archive_path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_a_same_version_reinstall_keeps_the_tree_it_extracted_over() {
        let (dir, db, downloader) = scratch("same_version");
        let pkg = package("same", "1.0.0", &[]);
        let downloaded = fake_download(&dir, &pkg);
        record(&db, &pkg, &downloaded, true);

        let mut session = Session::default();
        session.begin(&db, &pkg).unwrap();
        session.downloaded(&downloaded);
        record(&db, &pkg, &downloaded, true);
        session.recorded();

        session.unwind(&db, &downloader);

        assert_eq!(db.get_package("same").unwrap().version, "1.0.0");
        assert!(downloaded.extract_dir.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_packages_unwind_newest_first_and_untouched_ones_are_left_alone() {
        let (dir, db, downloader) = scratch("order");
        let dep = package("dep", "1.0.0", &[]);
        let root = package("root", "1.0.0", &["dep"]);

        let mut session = Session::default();
        session.begin(&db, &dep).unwrap();
        let dep_download = fake_download(&dir, &dep);
        session.downloaded(&dep_download);
        record(&db, &dep, &dep_download, false);
        session.recorded();
        // The root failed before anything happened to it.
        session.begin(&db, &root).unwrap();

        assert_eq!(session.installed(), vec!["dep".to_string()]);
        session.unwind(&db, &downloader);

        assert!(db.list_packages().unwrap().is_empty());
        assert!(!dep_download.extract_dir.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_an_empty_session_changes_nothing() {
        let (dir, db, downloader) = scratch("empty");
        let pkg = package("present", "1.0.0", &[]);
        let downloaded = fake_download(&dir, &pkg);
        record(&db, &pkg, &downloaded, true);

        let mut session = Session::default();
        session.begin(&db, &pkg).unwrap();
        session.unwind(&db, &downloader);

        assert_eq!(db.get_package("present").unwrap().version, "1.0.0");
        assert!(downloaded.extract_dir.exists());
        assert!(downloaded.archive_path.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
