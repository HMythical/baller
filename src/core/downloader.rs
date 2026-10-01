use std::fs::{self, File};
use std::io::{self, copy, Read, Write};
use std::path::{Path, PathBuf};

use indicatif::{ProgressBar, ProgressStyle};

use crate::core::package::{Package, PackageSource, Platform};
use crate::error::error::BallError;
use crate::http::HttpClient;
use crate::utils::{fs as util_fs, security};

pub struct Downloader {
    cache_dir: PathBuf,
    http_client: HttpClient,
}

#[derive(Debug, Clone)]
pub struct DownloadedPackage {
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

        // Layers above trust metadata; this checks the bytes, so it holds for
        // every source and every command that installs through here. `None`
        // is left to the caller's `no_binary_error`.
        let binary_path = match util_fs::find_binary_in_dir(&extract_dir, &pkg.name) {
            Some(candidate) => match judge_binary(&candidate, Platform::host())? {
                BinaryVerdict::Runnable => Some(candidate),
                BinaryVerdict::Empty => {
                    tracing::debug!(
                        "{}: binary candidate {} is empty, so there is nothing to run",
                        pkg.name,
                        candidate.display()
                    );
                    None
                }
                BinaryVerdict::Foreign(format) => {
                    return Err(self.platform_mismatch_error(
                        pkg,
                        &extract_dir,
                        &archive_path,
                        &candidate,
                        format,
                    ))
                }
            },
            None => None,
        };

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

    pub fn remove_extracted(&self, pkg: &Package) -> Result<(), BallError> {
        let extract_dir = self.cache_dir.join(format!("{}-{}", pkg.name, pkg.version));
        if extract_dir.exists() {
            fs::remove_dir_all(&extract_dir).map_err(BallError::FileIoErr)?;
        }
        Ok(())
    }

    /// The error for an extraction that produced no executable, with the cache
    /// cleaned out first.
    ///
    /// An extracted tree with no binary cannot be installed, so linking it,
    /// recording it or reporting success would all be lies — every command that
    /// extracts an archive routes that case through here. The extract directory
    /// and the cached archive are both removed so a retry re-downloads instead
    /// of reusing a package that produced nothing runnable; cleanup failures are
    /// reported at `--verbose` and never mask the real error.
    pub fn no_binary_error(&self, pkg: &Package, downloaded: &DownloadedPackage) -> BallError {
        let err = BallError::NoBinaryFound {
            package: pkg.name.clone(),
            version: pkg.version.clone(),
            dir: downloaded.extract_dir.to_string_lossy().to_string(),
            archive: Some(downloaded.archive_path.to_string_lossy().to_string()),
        };

        if let Err(cleanup_err) = self.remove_extracted(pkg) {
            tracing::debug!(
                "{}: could not remove extract dir {}: {}",
                pkg.name,
                downloaded.extract_dir.display(),
                cleanup_err
            );
        }

        if let Some(url) = pkg.download_url.as_deref() {
            if let Err(cleanup_err) = self.remove_archive(url) {
                tracing::debug!(
                    "{}: could not remove cached archive for {}: {}",
                    pkg.name,
                    url,
                    cleanup_err
                );
            }
        }

        err
    }

    /// The error for an extracted binary built for another platform, with the
    /// extract directory removed first.
    ///
    /// Unlike [`Downloader::no_binary_error`] the cached archive is kept: it
    /// downloaded intact and is exactly what the source serves, so fetching it
    /// again would only reproduce the mismatch. Cleanup failures are reported
    /// at `--verbose` and never mask the real error.
    fn platform_mismatch_error(
        &self,
        pkg: &Package,
        extract_dir: &Path,
        archive_path: &Path,
        binary: &Path,
        format: BinaryFormat,
    ) -> BallError {
        let err = BallError::PlatformMismatch {
            package: pkg.name.clone(),
            version: pkg.version.clone(),
            format: format.describe().to_string(),
            binary: binary.to_string_lossy().to_string(),
            archive: Some(archive_path.to_string_lossy().to_string()),
        };

        if let Err(cleanup_err) = self.remove_extracted(pkg) {
            tracing::debug!(
                "{}: could not remove extract dir {}: {}",
                pkg.name,
                extract_dir.display(),
                cleanup_err
            );
        }

        err
    }
}

/// What a binary candidate's first bytes say it is
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryFormat {
    /// `\x7fELF`: a Linux executable
    Elf,
    /// `MZ`: a PE/DOS executable, i.e. Windows
    Pe,
    /// `#!`: an interpreter script, runnable on either host
    Script,
    /// Zero bytes
    Empty,
    /// None of the above, so never a definite binary for either platform
    Unrecognized,
}

impl BinaryFormat {
    /// Classify `path` from its header.
    ///
    /// A read failure is an error, never "unrecognized": otherwise a
    /// permissions problem would pass the check silently.
    pub fn detect(path: &Path) -> Result<Self, BallError> {
        let mut header = Vec::with_capacity(4);
        File::open(path)
            .and_then(|file| file.take(4).read_to_end(&mut header))
            .map_err(BallError::FileIoErr)?;

        Ok(if header.is_empty() {
            BinaryFormat::Empty
        } else if header.starts_with(b"\x7fELF") {
            BinaryFormat::Elf
        } else if header.starts_with(b"MZ") {
            BinaryFormat::Pe
        } else if header.starts_with(b"#!") {
            BinaryFormat::Script
        } else {
            BinaryFormat::Unrecognized
        })
    }

    /// The platform a definite binary format runs on; `None` for the rest
    pub fn native_platform(&self) -> Option<Platform> {
        match self {
            BinaryFormat::Elf => Some(Platform::Linux),
            BinaryFormat::Pe => Some(Platform::Windows),
            BinaryFormat::Script | BinaryFormat::Empty | BinaryFormat::Unrecognized => None,
        }
    }

    pub fn describe(&self) -> &'static str {
        match self {
            BinaryFormat::Elf => "ELF/Linux executable",
            BinaryFormat::Pe => "PE/Windows executable",
            BinaryFormat::Script => "interpreter script",
            BinaryFormat::Empty => "empty file",
            BinaryFormat::Unrecognized => "unrecognized file",
        }
    }
}

/// Whether a binary candidate may be installed on a host
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinaryVerdict {
    /// Native, a script, or never a definite binary: install it
    Runnable,
    /// Nothing to run; treated as no binary found
    Empty,
    /// A definite binary for another platform: always fatal
    Foreign(BinaryFormat),
}

/// Judge a binary candidate for `host`.
///
/// Strict with one carve-out: a definite foreign binary is fatal, but a file
/// that was never a binary candidate passes. `find_executables` is a mode-bit
/// check on unix, so it legitimately returns `.ball` wrapper scripts and
/// other script-based packages; demanding ELF would break them.
fn judge_binary(path: &Path, host: Platform) -> Result<BinaryVerdict, BallError> {
    let format = BinaryFormat::detect(path)?;
    Ok(match format.native_platform() {
        Some(native) if native != host => BinaryVerdict::Foreign(format),
        _ if format == BinaryFormat::Empty => BinaryVerdict::Empty,
        _ => BinaryVerdict::Runnable,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// The first bytes of a real x86_64 ELF executable
    const ELF: &[u8] = b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x3e\x00";
    /// The first bytes of a real PE executable's DOS header
    const PE: &[u8] = b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xff\xff\x00\x00\xb8\x00";

    fn test_dir(tag: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("baller_test_downloader_{}_{}", tag, nanos));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    fn native() -> &'static [u8] {
        match Platform::host() {
            Platform::Linux => ELF,
            Platform::Windows => PE,
        }
    }

    fn foreign() -> &'static [u8] {
        match Platform::host() {
            Platform::Linux => PE,
            Platform::Windows => ELF,
        }
    }

    #[test]
    fn test_detect_classifies_headers() {
        let dir = test_dir("detect");
        let cases: [(&str, &[u8], BinaryFormat); 6] = [
            ("elf", ELF, BinaryFormat::Elf),
            ("pe", PE, BinaryFormat::Pe),
            ("script", b"#!/bin/sh\necho hi\n", BinaryFormat::Script),
            ("empty", b"", BinaryFormat::Empty),
            ("text", b"binary contents", BinaryFormat::Unrecognized),
            ("short", b"M", BinaryFormat::Unrecognized),
        ];
        for (name, contents, expected) in cases {
            let path = write_file(&dir, name, contents);
            assert_eq!(BinaryFormat::detect(&path).unwrap(), expected, "{}", name);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_unreadable_candidate_is_an_io_error_not_a_pass() {
        let dir = test_dir("unreadable");

        let missing = dir.join("missing");
        assert!(matches!(
            BinaryFormat::detect(&missing),
            Err(BallError::FileIoErr(_))
        ));
        assert!(judge_binary(&missing, Platform::host()).is_err());

        // A directory opens on unix but cannot be read as a file
        let subdir = dir.join("subdir");
        fs::create_dir_all(&subdir).unwrap();
        assert!(matches!(
            BinaryFormat::detect(&subdir),
            Err(BallError::FileIoErr(_))
        ));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_judge_binary_on_both_hosts() {
        let dir = test_dir("judge");
        let elf = write_file(&dir, "elf", ELF);
        let pe = write_file(&dir, "pe", PE);
        let script = write_file(&dir, "script", b"#!/usr/bin/env bash\n");
        let text = write_file(&dir, "text", b"binary contents");
        let empty = write_file(&dir, "empty", b"");

        let linux = Platform::Linux;
        let windows = Platform::Windows;
        assert_eq!(judge_binary(&elf, linux).unwrap(), BinaryVerdict::Runnable);
        assert_eq!(
            judge_binary(&elf, windows).unwrap(),
            BinaryVerdict::Foreign(BinaryFormat::Elf)
        );
        assert_eq!(
            judge_binary(&pe, linux).unwrap(),
            BinaryVerdict::Foreign(BinaryFormat::Pe)
        );
        assert_eq!(judge_binary(&pe, windows).unwrap(), BinaryVerdict::Runnable);
        for host in [linux, windows] {
            assert_eq!(
                judge_binary(&script, host).unwrap(),
                BinaryVerdict::Runnable
            );
            assert_eq!(judge_binary(&text, host).unwrap(), BinaryVerdict::Runnable);
            assert_eq!(judge_binary(&empty, host).unwrap(), BinaryVerdict::Empty);
        }

        let _ = fs::remove_dir_all(&dir);
    }

    /// Seed the cache with a zip holding `entry`, so `download_and_extract`
    /// runs end to end without touching the network.
    fn seeded(tag: &str, entry: &str, contents: &[u8]) -> (PathBuf, Downloader, Package) {
        let dir = test_dir(tag);
        let cache_dir = dir.join("cache");
        fs::create_dir_all(&cache_dir).unwrap();

        let url = format!("https://example.test/{}.zip", tag);
        let archive = cache_dir.join(util_fs::sanitize_filename(&url));
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file(entry, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents).unwrap();
        zip.finish().unwrap();

        let mut pkg = Package::new("tool", "1.0.0");
        pkg.download_url = Some(url);

        let downloader = Downloader::new(cache_dir, HttpClient::new().unwrap());
        (dir, downloader, pkg)
    }

    #[test]
    fn test_download_and_extract_accepts_a_native_binary() {
        let (dir, downloader, pkg) = seeded("native", "tool", native());
        let downloaded = downloader.download_and_extract(&pkg, false).unwrap();
        assert_eq!(
            downloaded.binary_path,
            Some(downloaded.extract_dir.join("tool"))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_download_and_extract_accepts_a_script_package() {
        // zip extraction leaves no exec bit; the exact-name probe still finds it
        let (dir, downloader, pkg) = seeded("script", "tool", b"#!/bin/sh\nexec true\n");
        let downloaded = downloader.download_and_extract(&pkg, false).unwrap();
        assert!(downloaded.binary_path.is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_download_and_extract_reports_an_empty_binary_as_none() {
        let (dir, downloader, pkg) = seeded("empty", "tool", b"");
        let downloaded = downloader.download_and_extract(&pkg, false).unwrap();
        assert_eq!(downloaded.binary_path, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_download_and_extract_rejects_a_foreign_binary_and_keeps_the_archive() {
        let (dir, downloader, pkg) = seeded("foreign", "tool", foreign());
        let archive = downloader.cache_dir.join(util_fs::sanitize_filename(
            pkg.download_url.as_deref().unwrap(),
        ));
        let extract_dir = downloader.cache_dir.join("tool-1.0.0");

        match downloader.download_and_extract(&pkg, false) {
            Err(BallError::PlatformMismatch {
                package,
                version,
                format,
                binary,
                archive: reported,
            }) => {
                assert_eq!(package, "tool");
                assert_eq!(version, "1.0.0");
                assert!(format.contains("executable"));
                assert!(binary.ends_with("tool"));
                assert_eq!(reported.as_deref(), archive.to_str());
            }
            other => panic!("expected PlatformMismatch, got {:?}", other),
        }

        assert!(!extract_dir.exists(), "extract dir must be removed");
        assert!(archive.is_file(), "cached archive must survive");
        let _ = fs::remove_dir_all(&dir);
    }
}
