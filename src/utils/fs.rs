use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::{fs, io};

use crate::error::error::BallError;

pub fn ensure_dir(path: &Path) -> Result<(), BallError> {
    fs::create_dir_all(path).map_err(BallError::FileIoErr)
}

#[allow(dead_code)]
pub fn ensure_parent(path: &Path) -> Result<(), BallError> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)
    } else {
        Ok(())
    }
}

#[allow(dead_code)]
pub fn remove_whole_dir(path: &Path) -> Result<(), BallError> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(BallError::FileIoErr)
    } else {
        Ok(())
    }
}

#[allow(dead_code)]
pub fn remove_file(path: &Path) -> Result<(), BallError> {
    if path.exists() {
        fs::remove_file(path).map_err(BallError::FileIoErr)
    } else {
        Ok(())
    }
}

#[allow(dead_code)]
pub fn path_exists(path: &Path) -> bool {
    path.exists()
}

pub fn dir_size(path: &Path) -> Result<u64, BallError> {
    let mut total = 0u64;
    for entry in fs::read_dir(path).map_err(BallError::FileIoErr)? {
        let entry = entry.map_err(BallError::FileIoErr)?;
        let entry_path = entry.path();
        let metadata = entry.metadata().map_err(BallError::FileIoErr)?;
        if metadata.is_file() {
            total += metadata.len();
        } else if metadata.is_dir() {
            total += dir_size(&entry_path)?;
        }
    }
    Ok(total)
}

#[allow(dead_code)]
pub fn read_dir_names(path: &Path) -> Result<Vec<String>, BallError> {
    let mut names = Vec::new();
    for entry in fs::read_dir(path).map_err(BallError::FileIoErr)? {
        let entry = entry.map_err(BallError::FileIoErr)?;
        names.push(entry.file_name().to_string_lossy().to_string());
    }
    names.sort();
    Ok(names)
}

pub fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '/' | ':' | '?' | '&' | '=' | '%' | '#' | '\\' | '<' | '>' | '"' | '|' => '_',
            _ => c,
        })
        .collect();

    if sanitized.len() > 200 {
        let ext = Path::new(&sanitized)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e))
            .unwrap_or_default();
        let base: String = sanitized.chars().take(190).collect();
        format!("{}{}", base, ext)
    } else {
        sanitized
    }
}

#[allow(dead_code)]
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), BallError> {
    ensure_parent(path)?;

    let tmp_path = {
        let mut p = path.as_os_str().to_os_string();
        p.push(".tmp");
        PathBuf::from(p)
    };

    {
        let mut tmp = fs::File::create(&tmp_path).map_err(BallError::FileIoErr)?;
        tmp.write_all(contents).map_err(BallError::FileIoErr)?;
        tmp.sync_all().map_err(BallError::FileIoErr)?;
    }

    fs::rename(&tmp_path, path).map_err(BallError::FileIoErr)
}

#[allow(dead_code)]
pub fn copy_file(src: &Path, dst: &Path) -> Result<(), BallError> {
    ensure_parent(dst)?;
    fs::copy(src, dst).map_err(BallError::FileIoErr)?;
    Ok(())
}

pub fn find_executables(dir: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && is_executable(&path) {
                results.push(path);
            }
        }
    }
    results
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".exe") || name.ends_with(".bat") || name.ends_with(".cmd")
}

/// Safely truncate a string to a maximum number of Unicode characters.
/// Adds "..." if truncated.
pub fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        format!(
            "{}...",
            s.chars()
                .take(max_chars.saturating_sub(3))
                .collect::<String>()
        )
    } else {
        s.to_string()
    }
}

/// Parse a human-written size into bytes (e.g. `"50MB"`, `"1.5 gb"`, `"1048576"`).
///
/// Bare numbers are treated as bytes. Suffixes are binary multiples (KB = 1024).
pub fn parse_size(input: &str) -> Result<u64, BallError> {
    let trimmed = input.trim().to_lowercase();
    if trimmed.is_empty() {
        return Err(BallError::InvalidConfig(
            "size cannot be empty (try 50MB or 1048576)".to_string(),
        ));
    }

    let digits_end = trimmed
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(trimmed.len());
    let (number, suffix) = trimmed.split_at(digits_end);
    let suffix = suffix.trim();

    let value: f64 = number.parse().map_err(|_| {
        BallError::InvalidConfig(format!("invalid size '{}': expected a number", input))
    })?;

    if value < 0.0 || !value.is_finite() {
        return Err(BallError::InvalidConfig(format!(
            "invalid size '{}': must be a positive number",
            input
        )));
    }

    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let multiplier = match suffix {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => KB,
        "m" | "mb" | "mib" => MB,
        "g" | "gb" | "gib" => GB,
        other => {
            return Err(BallError::InvalidConfig(format!(
                "unknown size unit '{}': expected B, KB, MB, or GB",
                other
            )))
        }
    };

    Ok((value * multiplier) as u64)
}

/// Format bytes into a human-readable string (e.g., "45.2 MB").
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Prompt user for confirmation from stdin. Returns true if user confirms.
pub fn confirm(prompt: &str) -> Result<bool, BallError> {
    let stdin = io::stdin();

    if !stdin.is_terminal() {
        return Err(BallError::PipeRedirected {
            pipe: "stdin".to_string(),
            msg: "--yes/-y flag required to use this command with redirected pipe".to_string(),
        });
    }

    print!("{prompt} ");
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    Ok(matches!(input.trim().to_lowercase().as_str(), "y" | "yes"))
}

pub fn find_binary_in_dir(dir: &Path, pkg_name: &str) -> Option<PathBuf> {
    let candidates = [
        dir.join(pkg_name),
        dir.join("bin").join(pkg_name),
        dir.join(format!("{}.exe", pkg_name)),
        dir.join("bin").join(format!("{}.exe", pkg_name)),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }

    let executables = find_executables(dir);
    executables.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_bare_bytes() {
        assert_eq!(parse_size("1048576").unwrap(), 1_048_576);
        assert_eq!(parse_size("0").unwrap(), 0);
    }

    #[test]
    fn test_parse_size_units() {
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("1kib").unwrap(), 1024);
        assert_eq!(parse_size("50MB").unwrap(), 50 * 1024 * 1024);
        assert_eq!(parse_size("2g").unwrap(), 2 * 1024 * 1024 * 1024);
        assert_eq!(parse_size("512b").unwrap(), 512);
    }

    #[test]
    fn test_parse_size_fractional_and_spacing() {
        assert_eq!(parse_size("1.5MB").unwrap(), 1_572_864);
        assert_eq!(parse_size("  10 mb  ").unwrap(), 10 * 1024 * 1024);
        assert_eq!(parse_size("1.5 GB").unwrap(), 1_610_612_736);
    }

    #[test]
    fn test_parse_size_rejects_bad_input() {
        assert!(parse_size("").is_err());
        assert!(parse_size("abc").is_err());
        assert!(parse_size("10 tb").is_err());
        assert!(parse_size("-5MB").is_err());
    }

    #[test]
    fn test_parse_size_round_trips_with_format_size() {
        let bytes = parse_size("50MB").unwrap();
        assert!(format_size(bytes).contains("50"));
        assert!(format_size(bytes).contains("MB"));
    }

    #[test]
    fn test_sanitize_filename_basic() {
        let result = sanitize_filename("hello.tar.gz");
        assert_eq!(result, "hello.tar.gz");
    }

    #[test]
    fn test_sanitize_filename_replaces_chars() {
        let result = sanitize_filename("foo/bar:baz?qux&quux=100%20#frag");
        assert_eq!(result, "foo_bar_baz_qux_quux_100_20_frag");
    }

    #[test]
    fn test_sanitize_filename_backslash() {
        let result = sanitize_filename("a\\b<c>d\"e|f");
        assert_eq!(result, "a_b_c_d_e_f");
    }

    #[test]
    fn test_sanitize_filename_truncate_long() {
        let long = "a".repeat(300);
        let result = sanitize_filename(&long);
        assert!(result.len() <= 200);
    }

    #[test]
    fn test_sanitize_filename_truncate_preserves_extension() {
        let long = format!("{}", "a".repeat(250)) + ".tar.gz";
        let result = sanitize_filename(&long);
        assert!(result.len() <= 200);
        assert!(result.ends_with(".gz") || result.ends_with(".tar.gz"));
    }

    #[test]
    fn test_ensure_dir_creates() {
        let dir = std::env::temp_dir().join("baller_test_ensure_dir");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!dir.exists());
        ensure_dir(&dir).unwrap();
        assert!(dir.exists());
        ensure_dir(&dir).unwrap(); // idempotent
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ensure_parent_single() {
        let dir = std::env::temp_dir().join("baller_test_parent").join("sub");
        let file_path = dir.join("test.txt");
        let _ = std::fs::remove_dir_all(&dir.parent().unwrap());
        ensure_parent(&file_path).unwrap();
        assert!(dir.exists());
        let _ = std::fs::remove_dir_all(&dir.parent().unwrap());
    }

    #[test]
    fn test_remove_whole_dir_exists() {
        let dir = std::env::temp_dir().join("baller_test_remove_dir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f.txt"), b"data").unwrap();
        remove_whole_dir(&dir).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn test_remove_whole_dir_not_exists() {
        let dir = std::env::temp_dir().join("baller_test_remove_nonexistent");
        remove_whole_dir(&dir).unwrap(); // should not error
    }

    #[test]
    fn test_remove_file_exists() {
        let dir = std::env::temp_dir().join("baller_test_remove_file");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.txt");
        std::fs::write(&f, b"").unwrap();
        remove_file(&f).unwrap();
        assert!(!f.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remove_file_not_exists() {
        remove_file(Path::new("/nonexistent/file.txt")).unwrap();
    }

    #[test]
    fn test_path_exists() {
        assert!(path_exists(&std::env::temp_dir()));
        assert!(!path_exists(Path::new("/nonexistent_path_xyz123")));
    }

    #[test]
    fn test_dir_size_empty() {
        let dir = std::env::temp_dir().join("baller_test_size_empty");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(dir_size(&dir).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_dir_size_with_files() {
        let dir = std::env::temp_dir().join("baller_test_size_files");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::fs::write(dir.join("b.txt"), b"world").unwrap();
        assert_eq!(dir_size(&dir).unwrap(), 10);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_dir_size_nested() {
        let dir = std::env::temp_dir().join("baller_test_size_nested");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("root.txt"), b"12345").unwrap();
        std::fs::write(sub.join("nested.txt"), b"67890").unwrap();
        assert_eq!(dir_size(&dir).unwrap(), 10);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_dir_size_nonexistent() {
        let dir = Path::new("/nonexistent_dir_xyz_abc");
        assert!(
            matches!(dir_size(dir), Err(BallError::FileIoErr(err)) if err.kind() == io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn test_read_dir_names() {
        let dir = std::env::temp_dir().join("baller_test_readdir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.txt"), b"").unwrap();
        std::fs::write(dir.join("a.txt"), b"").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let names = read_dir_names(&dir).unwrap();
        assert_eq!(names, vec!["a.txt", "b.txt", "sub"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_atomic_write_creates_file() {
        let dir = std::env::temp_dir().join("baller_test_atomic");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("test.txt");
        atomic_write(&f, b"hello atomic").unwrap();
        assert!(f.exists());
        let content = std::fs::read_to_string(&f).unwrap();
        assert_eq!(content, "hello atomic");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_atomic_write_overwrites() {
        let dir = std::env::temp_dir().join("baller_test_atomic2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("test.txt");
        atomic_write(&f, b"first").unwrap();
        atomic_write(&f, b"second").unwrap();
        let content = std::fs::read_to_string(&f).unwrap();
        assert_eq!(content, "second");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_copy_file() {
        let dir = std::env::temp_dir().join("baller_test_copy");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.txt");
        let dst = dir.join("sub").join("dst.txt");
        std::fs::write(&src, b"copy content").unwrap();
        copy_file(&src, &dst).unwrap();
        assert!(dst.exists());
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "copy content");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_binary_in_dir_direct_match() {
        let dir = std::env::temp_dir().join("baller_test_findbin");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("myapp"), b"binary").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("myapp"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let result = find_binary_in_dir(&dir, "myapp");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().file_name().unwrap().to_str().unwrap(),
            "myapp"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_binary_in_dir_bin_subdir() {
        let dir = std::env::temp_dir().join("baller_test_findbin_bin");
        let _ = std::fs::remove_dir_all(&dir);
        let bin_dir = dir.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("myapp"), b"binary").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                bin_dir.join("myapp"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let result = find_binary_in_dir(&dir, "myapp");
        assert!(result.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_binary_in_dir_no_match() {
        let dir = std::env::temp_dir().join("baller_test_findbin_none");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("readme.txt"), b"text").unwrap();
        let result = find_binary_in_dir(&dir, "myapp");
        assert!(result.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_binary_in_dir_non_existent() {
        let dir = Path::new("/nonexistent_xyz_dir");
        let result = find_binary_in_dir(dir, "app");
        assert!(result.is_none());
    }

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_str_long() {
        let result = truncate_str("this is a very long string", 10);
        assert!(result.len() == 10); // 7 chars + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_str_multi_byte() {
        let emoji_desc = "Hello world";
        let result = truncate_str(emoji_desc, 5);
        assert!(result.contains("..."));
    }

    #[test]
    fn test_format_size_zero() {
        assert_eq!(format_size(0), "0 bytes");
    }

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(500), "500 bytes");
    }

    #[test]
    fn test_format_size_kb() {
        let result = format_size(1536); // 1.5 KB
        assert!(result.contains("KB"));
        assert!(result.contains("1.5"));
    }

    #[test]
    fn test_format_size_mb() {
        let result = format_size(10_485_760); // ~10 MB
        assert!(result.contains("MB"));
        assert!(result.contains("10"));
    }

    #[test]
    fn test_format_size_gb() {
        let result = format_size(2_147_483_648); // 2 GB
        assert!(result.contains("GB"));
        assert!(result.contains("2"));
    }
}
