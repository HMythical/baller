pub mod chocolatey;
pub mod github;
pub mod registry_api;

use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};

use crate::error::error::BallError;

const BALLER_USER_AGENT: &str = concat!("baller/", env!("CARGO_PKG_VERSION"));
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_RETRIES: u32 = 3;

#[derive(Clone)]
pub struct HttpClient {
    client: Client,
}

impl HttpClient {
    pub fn new() -> Result<Self, BallError> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(BALLER_USER_AGENT));

        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| BallError::NetworkError(format!("failed to create HTTP client: {}", e)))?;

        Ok(Self { client })
    }

    pub fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, BallError> {
        self.get_json_with_accept(url, "application/json")
    }

    pub fn get_json_with_accept<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        accept: &str,
    ) -> Result<T, BallError> {
        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            let mut req = self.client.get(url);
            if let Ok(accept_val) = HeaderValue::from_str(accept) {
                req = req.header(ACCEPT, accept_val);
            }

            match req.send() {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        match resp.json::<T>() {
                            Ok(data) => return Ok(data),
                            Err(e) => {
                                return Err(BallError::NetworkError(format!(
                                    "failed to parse JSON response from {}: {}",
                                    url, e
                                )));
                            }
                        }
                    } else if status.as_u16() == 404 {
                        return Err(BallError::PackageNotFound(url.to_string()));
                    } else if status.as_u16() == 403 {
                        return Err(BallError::NetworkError(format!(
                            "rate limited or forbidden when accessing {}: {}",
                            url, status
                        )));
                    } else {
                        last_error = Some(BallError::NetworkError(format!(
                            "HTTP {} when accessing {}",
                            status, url
                        )));
                    }
                }
                Err(e) => {
                    last_error = Some(BallError::NetworkError(format!(
                        "request to {} failed (attempt {}): {}",
                        url,
                        attempt + 1,
                        e
                    )));
                    if attempt < MAX_RETRIES - 1 {
                        std::thread::sleep(Duration::from_secs(1 << attempt));
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            BallError::NetworkError(format!("all retries exhausted for {}", url))
        }))
    }

    #[allow(dead_code)]
    pub fn get_text(&self, url: &str) -> Result<String, BallError> {
        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            match self.client.get(url).send() {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return resp.text().map_err(|e| {
                            BallError::NetworkError(format!("failed to read response body: {}", e))
                        });
                    } else if status.as_u16() == 404 {
                        return Err(BallError::PackageNotFound(url.to_string()));
                    } else {
                        last_error = Some(BallError::NetworkError(format!(
                            "HTTP {} when accessing {}",
                            status, url
                        )));
                    }
                }
                Err(e) => {
                    last_error = Some(BallError::NetworkError(format!(
                        "request to {} failed (attempt {}): {}",
                        url,
                        attempt + 1,
                        e
                    )));
                    if attempt < MAX_RETRIES - 1 {
                        std::thread::sleep(Duration::from_secs(1 << attempt));
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            BallError::NetworkError(format!("all retries exhausted for {}", url))
        }))
    }

    pub fn download_to<W: std::io::Write>(
        &self,
        url: &str,
        writer: &mut W,
    ) -> Result<u64, BallError> {
        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            match self.client.get(url).send() {
                Ok(mut resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let bytes = resp.copy_to(writer).map_err(|e| {
                            BallError::NetworkError(format!("download failed: {}", e))
                        })?;
                        return Ok(bytes);
                    } else if status.as_u16() == 404 {
                        return Err(BallError::PackageNotFound(url.to_string()));
                    } else {
                        last_error = Some(BallError::NetworkError(format!(
                            "HTTP {} when downloading {}",
                            status, url
                        )));
                    }
                }
                Err(e) => {
                    last_error = Some(BallError::NetworkError(format!(
                        "download from {} failed (attempt {}): {}",
                        url,
                        attempt + 1,
                        e
                    )));
                    if attempt < MAX_RETRIES - 1 {
                        std::thread::sleep(Duration::from_secs(1 << attempt));
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            BallError::NetworkError(format!("all retries exhausted for {}", url))
        }))
    }

    pub fn get_response(&self, url: &str) -> Result<reqwest::blocking::Response, BallError> {
        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            match self.client.get(url).send() {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(resp);
                    } else if status.as_u16() == 404 {
                        return Err(BallError::PackageNotFound(url.to_string()));
                    } else {
                        last_error = Some(BallError::NetworkError(format!(
                            "HTTP {} when accessing {}",
                            status, url
                        )));
                    }
                }
                Err(e) => {
                    last_error = Some(BallError::NetworkError(format!(
                        "request to {} failed (attempt {}): {}",
                        url,
                        attempt + 1,
                        e
                    )));
                    if attempt < MAX_RETRIES - 1 {
                        std::thread::sleep(Duration::from_secs(1 << attempt));
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            BallError::NetworkError(format!("all retries exhausted for {}", url))
        }))
    }
}
