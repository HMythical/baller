use std::fs::{self, File};
use std::io::{self, copy, Read, Write};
use std::path::{Path, PathBuf};

use indicatif::{ProgressBar, ProgressStyle};

use crate::core::package::{Package, PackageSource};
use crate::error::error::BallError;
use crate::http::HttpClient;
use crate::utils::{fs as util_fs, security};

pub struct Downloader {
    cache_dir: PathBuf,
    http_client: HttpClient,
}

#[derive(Debug, Clone)]
pub struct DownloadedPackage {
    #[allow(dead_code)]
    pub archive_path: PathBuf,
    pub extract_dir: PathBuf,
    pub binary_path: Option<PathBuf>,
}

impl Downloader {
    pub fn new(cache_dir: PathBuf, http_client: HttpClient) -> Self {
        Self {
            cache_dir,
            http_client,
        }
    }

    pub fn download_and_extract(
        &self,
        pkg: &Package,
        show_progress: bool,
    ) -> Result<DownloadedPackage, BallError> {
        // System packages don't have download URLs
        if pkg.download_url.is_none() {
            if matches!(pkg.source, PackageSource::System { .. }) {
                return Err(BallError::PackageManagerError(format!(
                    "'{}' is a system package — use native package manager",
                    pkg.name
                )));
            }
            if matches!(pkg.source, PackageSource::Cargo { .. }) {
                return Err(BallError::PackageManagerError(format!(
                    "'{}' is a cargo package — use cargo install",
                    pkg.name
                )));
            }
            return Err(BallError::NetworkError(format!(
                "no download URL for package '{}'",
                pkg.name
            )));
        }

        let url = pkg.download_url.as_ref().ok_or_else(|| {
            BallError::NetworkError(format!("no download URL for package '{}'", pkg.name))
        })?;

        let archive_path = self.cached_download(url, show_progress)?;

        if let Some(ref expected_hash) = pkg.sha256 {
            let algorithm = pkg.hash_algorithm.as_deref().unwrap_or("SHA256");

            if algorithm.eq_ignore_ascii_case("SHA512") || algorithm.eq_ignore_ascii_case("SHA-512")
            {
                use base64::Engine;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(expected_hash)
                    .map_err(|e| BallError::HashMismatch(format!("invalid base64 hash: {}", e)))?;
                let hex_hash = hex::encode(decoded);
                security::verify_checksum_with_algorithm(&archive_path, &hex_hash, "SHA512")?;
            } else {
                security::verify_checksum(&archive_path, expected_hash)?;
            }
        }

        let extract_dir = self.cache_dir.join(format!("{}-{}", pkg.name, pkg.version));
        if extract_dir.exists() {
            fs::remove_dir_all(&extract_dir).map_err(BallError::FileIoErr)?;
        }
        fs::create_dir_all(&extract_dir).map_err(BallError::FileIoErr)?;

        self.extract_archive(&archive_path, &extract_dir)?;

        let binary_path = util_fs::find_binary_in_dir(&extract_dir, &pkg.name);

        Ok(DownloadedPackage {
            archive_path,
            extract_dir,
            binary_path,
        })
    }

    pub fn download_archive(
        &self,
        url: &str,
        dest: &Path,
        show_progress: bool,
    ) -> Result<(), BallError> {
        if show_progress {
            self.download_with_progress(url, dest)
        } else {
            let mut file = File::create(dest).map_err(BallError::FileIoErr)?;
            self.http_client.download_to(url, &mut file)?;
            Ok(())
        }
    }

    fn download_with_progress(&self, url: &str, dest: &Path) -> Result<(), BallError> {
        let resp = self.http_client.get_response(url)?;

        let total_size = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        let pb = match total_size {
            Some(size) => {
                let pb = ProgressBar::new(size);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("{msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                        .map_err(|e| {
                            BallError::NetworkError(format!("progress bar template: {}", e))
                        })?
                        .progress_chars("=> "),
                );
                pb
            }
            None => {
                let pb = ProgressBar::new_spinner();
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("{spinner} {msg} {bytes}")
                        .map_err(|e| BallError::NetworkError(format!("spinner template: {}", e)))?,
                );
                pb
            }
        };

        pb.set_message(format!(
            "Downloading {}",
            dest.file_name().unwrap_or_default().to_string_lossy()
        ));

        let mut file = File::create(dest).map_err(BallError::FileIoErr)?;
        let mut source = resp.take(total_size.unwrap_or(u64::MAX));
        let mut buf = [0; 8192];
        let mut downloaded: u64 = 0;

        loop {
            let n = source
                .read(&mut buf)
                .map_err(|e| BallError::NetworkError(format!("download read error: {}", e)))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(BallError::FileIoErr)?;
            downloaded += n as u64;
            pb.set_position(downloaded);
        }

        pb.finish_with_message(format!(
            "Downloaded {}",
            dest.file_name().unwrap_or_default().to_string_lossy()
        ));
        Ok(())
    }

    fn cached_download(&self, url: &str, show_progress: bool) -> Result<PathBuf, BallError> {
        fs::create_dir_all(&self.cache_dir).map_err(BallError::FileIoErr)?;

        let filename = util_fs::sanitize_filename(url);
        let dest = self.cache_dir.join(&filename);

        if dest.exists() {
            return Ok(dest);
        }

        self.download_archive(url, &dest, show_progress)?;
        Ok(dest)
    }

    pub fn extract_archive(&self, archive_path: &Path, dest: &Path) -> Result<(), BallError> {
        let filename = archive_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
            extract_tar_gz(archive_path, dest)?;
        } else if filename.ends_with(".tar.bz2") || filename.ends_with(".tbz2") {
            extract_tar_bz2(archive_path, dest)?;
        } else if filename.ends_with(".tar.xz") || filename.ends_with(".txz") {
            extract_tar_xz(archive_path, dest)?;
        } else if filename.ends_with(".tar") {
            extract_tar(archive_path, dest)?;
        } else if filename.ends_with(".zip") || filename.ends_with(".nupkg") {
            extract_zip(archive_path, dest)?;
        } else if filename.ends_with(".gz") {
            extract_gz_single(archive_path, dest)?;
        } else {
            // Fallback: try zip extraction for archives without a recognized extension
            // (e.g., Chocolatey nupkg files cached from API URLs)
            extract_zip(archive_path, dest).map_err(|e| {
                BallError::ExtractionFailed(format!(
                    "unsupported archive format ({}): {}",
                    filename, e
                ))
            })?;
        }

        Ok(())
    }

    /// Wipe the whole cache, including the extracted `<name>-<version>` trees
    /// that installed binaries are linked against.
    pub fn cleanup_all(&self) -> Result<(), BallError> {
        if self.cache_dir.exists() {
            fs::remove_dir_all(&self.cache_dir).map_err(BallError::FileIoErr)?;
        }
        fs::create_dir_all(&self.cache_dir).map_err(BallError::FileIoErr)?;
        Ok(())
    }

    /// Downloaded archives sitting at the top level of the cache directory.
    ///
    /// Extracted packages live in subdirectories, so listing plain files here
    /// yields exactly the re-downloadable archives.
    pub fn archive_paths(&self) -> Vec<PathBuf> {
        let mut archives = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    archives.push(path);
                }
            }
        }
        archives.sort();
        archives
    }

    /// Delete cached archives while preserving extracted package directories.
    ///
    /// Returns the number of archives removed and the bytes reclaimed.
    pub fn cleanup_archives(&self) -> Result<(u64, u64), BallError> {
        let mut removed = 0u64;
        let mut freed = 0u64;

        for archive in self.archive_paths() {
            let size = archive.metadata().map(|m| m.len()).unwrap_or(0);
            fs::remove_file(&archive).map_err(BallError::FileIoErr)?;
            removed += 1;
            freed += size;
        }

        Ok((removed, freed))
    }

    /// Remove the cached archive a package was downloaded from.
    ///
    /// Returns whether an archive was actually present.
    pub fn remove_archive(&self, download_url: &str) -> Result<bool, BallError> {
        let archive = self
            .cache_dir
            .join(util_fs::sanitize_filename(download_url));
        if !archive.is_file() {
            return Ok(false);
        }

        fs::remove_file(&archive).map_err(BallError::FileIoErr)?;
        Ok(true)
    }

    #[allow(dead_code)]
    pub fn remove_extracted(&self, pkg: &Package) -> Result<(), BallError> {
        let extract_dir = self.cache_dir.join(format!("{}-{}", pkg.name, pkg.version));
        if extract_dir.exists() {
            fs::remove_dir_all(&extract_dir).map_err(BallError::FileIoErr)?;
        }
        Ok(())
    }
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), BallError> {
    let file = File::open(archive).map_err(BallError::FileIoErr)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(dest)
        .map_err(|e| BallError::ExtractionFailed(format!("tar.gz extraction: {}", e)))
}

fn extract_tar_bz2(archive: &Path, dest: &Path) -> Result<(), BallError> {
    let file = File::open(archive).map_err(BallError::FileIoErr)?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(dest)
        .map_err(|e| BallError::ExtractionFailed(format!("tar.bz2 extraction: {}", e)))
}

fn extract_tar_xz(archive: &Path, dest: &Path) -> Result<(), BallError> {
    let file = File::open(archive).map_err(BallError::FileIoErr)?;
    let decoder = xz2::read::XzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(dest)
        .map_err(|e| BallError::ExtractionFailed(format!("tar.xz extraction: {}", e)))
}

fn extract_tar(archive: &Path, dest: &Path) -> Result<(), BallError> {
    let file = File::open(archive).map_err(BallError::FileIoErr)?;
    let mut archive = tar::Archive::new(file);
    archive
        .unpack(dest)
        .map_err(|e| BallError::ExtractionFailed(format!("tar extraction: {}", e)))
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), BallError> {
    let file = File::open(archive).map_err(BallError::FileIoErr)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| BallError::ExtractionFailed(format!("zip open: {}", e)))?;

    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| BallError::ExtractionFailed(format!("zip entry {}: {}", i, e)))?;
        let entry_path = entry.mangled_name();
        let full_path = dest.join(&entry_path);

        if entry.is_dir() {
            fs::create_dir_all(&full_path).map_err(BallError::FileIoErr)?;
        } else {
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).map_err(BallError::FileIoErr)?;
            }
            let mut outfile = File::create(&full_path).map_err(BallError::FileIoErr)?;
            io::copy(&mut entry, &mut outfile).map_err(BallError::FileIoErr)?;
        }
    }

    Ok(())
}

fn extract_gz_single(archive: &Path, dest: &Path) -> Result<(), BallError> {
    let file = File::open(archive).map_err(BallError::FileIoErr)?;
    let mut decoder = flate2::read::GzDecoder::new(file);

    let out_name = archive.file_stem().unwrap_or_default();
    let out_path = dest.join(out_name);

    let mut outfile = File::create(&out_path).map_err(BallError::FileIoErr)?;
    copy(&mut decoder, &mut outfile).map_err(BallError::FileIoErr)?;
    Ok(())
}
