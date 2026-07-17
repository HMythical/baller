use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256, Sha512};

use crate::error::error::BallError;

const HASH_READ_BUF_SIZE: usize = 65536;

pub fn sha256_file(path: &Path) -> Result<String, BallError> {
    let mut file = std::fs::File::open(path).map_err(BallError::FileIoErr)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; HASH_READ_BUF_SIZE];

    loop {
        let n = file.read(&mut buf).map_err(BallError::FileIoErr)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}

pub fn verify_checksum(path: &Path, expected_hex: &str) -> Result<(), BallError> {
    let actual_hex = sha256_file(path)?;
    if !actual_hex.eq_ignore_ascii_case(expected_hex) {
        return Err(BallError::HashMismatch(format!(
            "expected {}, got {}",
            expected_hex, actual_hex
        )));
    }
    Ok(())
}

pub fn sha512_file(path: &Path) -> Result<String, BallError> {
    let mut file = std::fs::File::open(path).map_err(BallError::FileIoErr)?;
    let mut hasher = Sha512::new();
    let mut buf = [0u8; HASH_READ_BUF_SIZE];

    loop {
        let n = file.read(&mut buf).map_err(BallError::FileIoErr)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hex::encode(hasher.finalize()))
}

pub fn verify_checksum_with_algorithm(
    path: &Path,
    expected_hash: &str,
    algorithm: &str,
) -> Result<(), BallError> {
    let actual_hex = match algorithm.to_uppercase().as_str() {
        "SHA256" | "SHA-256" => sha256_file(path)?,
        "SHA512" | "SHA-512" => sha512_file(path)?,
        _ => {
            return Err(BallError::HashMismatch(format!(
                "unsupported hash algorithm: {}",
                algorithm
            )));
        }
    };
    if !actual_hex.eq_ignore_ascii_case(expected_hash) {
        return Err(BallError::HashMismatch(format!(
            "expected {}, got {}",
            expected_hash, actual_hex
        )));
    }
    Ok(())
}

#[allow(dead_code)]
pub fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[allow(dead_code)]
pub fn sha256_str(s: &str) -> String {
    sha256_bytes(s.as_bytes())
}

#[allow(dead_code)]
pub fn short_hash(s: &str, len: usize) -> String {
    let full = sha256_str(s);
    full.chars().take(len).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_sha256_bytes_known_value() {
        let result = sha256_bytes(b"hello");
        assert_eq!(result.len(), 64);
        assert!(result.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sha256_str_consistency() {
        let a = sha256_str("test value");
        let b = sha256_str("test value");
        assert_eq!(a, b);
    }

    #[test]
    fn test_sha256_str_different() {
        let a = sha256_str("foo");
        let b = sha256_str("bar");
        assert_ne!(a, b);
    }

    #[test]
    fn test_sha256_empty_string() {
        let result = sha256_str("");
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn test_short_hash_truncates() {
        let result = short_hash("hello world", 8);
        assert_eq!(result.len(), 8);
    }

    #[test]
    fn test_short_hash_zero_len() {
        let result = short_hash("anything", 0);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_sha256_file_roundtrip() -> Result<(), BallError> {
        let dir = std::env::temp_dir().join("baller_test_security");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let file_path = dir.join("test.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(b"hello world").unwrap();
        drop(f);

        let hash = sha256_file(&file_path)?;
        assert_eq!(hash.len(), 64);

        verify_checksum(&file_path, &hash)?;

        let result = verify_checksum(
            &file_path,
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn test_verify_checksum_case_insensitive() -> Result<(), BallError> {
        let dir = std::env::temp_dir().join("baller_test_security_case");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let file_path = dir.join("test.txt");
        std::fs::write(&file_path, b"data").unwrap();

        let hash = sha256_file(&file_path)?;
        let upper = hash.to_uppercase();
        verify_checksum(&file_path, &upper)?;

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn test_sha256_file_not_found() {
        let result = sha256_file(Path::new("/nonexistent/file"));
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_checksum_mismatch() -> Result<(), BallError> {
        let dir = std::env::temp_dir().join("baller_test_mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let file_path = dir.join("test.txt");
        std::fs::write(&file_path, b"content").unwrap();

        let wrong_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let result = verify_checksum(&file_path, wrong_hash);
        assert!(result.is_err());
        match result.unwrap_err() {
            BallError::HashMismatch(msg) => assert!(msg.contains("expected")),
            _ => panic!("expected HashMismatch error"),
        }

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
