use crate::config::config::BallerConfig;
use crate::core::db::DbManager;
use crate::core::downloader::Downloader;
use crate::core::registry::{RegistryClient, RegistrySource};
use crate::error::error::BallError;
use crate::http::HttpClient;
use crate::utils::fs::ensure_dir;

pub struct AppContext {
    pub config: BallerConfig,
    pub db: DbManager,
    #[allow(dead_code)]
    pub http_client: HttpClient,
    pub registry: RegistryClient,
    pub downloader: Downloader,
}

impl AppContext {
    pub fn new(config: BallerConfig) -> Result<Self, BallError> {
        ensure_dir(&config.hooks_dir)?;
        ensure_dir(&config.cache_dir)?;

        let http_client = HttpClient::new()?;
        let db = DbManager::init_at_path(&config.db_path)?;
        let downloader = Downloader::new(config.cache_dir.clone(), http_client.clone());

        let effective_order: Vec<RegistrySource> = config
            .registry
            .source_order
            .iter()
            .filter_map(|s| {
                let enabled = match s.as_str() {
                    "github" => config.registry.github_enabled,
                    "baller" => config.registry.baller_enabled,
                    "chocolatey" => config.registry.chocolatey_enabled,
                    _ => true,
                };
                if enabled {
                    match s.as_str() {
                        "github" => Some(RegistrySource::GitHub),
                        "baller" => Some(RegistrySource::BallerRegistry),
                        "chocolatey" => Some(RegistrySource::Chocolatey),
                        _ => Some(RegistrySource::GitHub),
                    }
                } else {
                    None
                }
            })
            .collect();

        let registry = RegistryClient::with_source_order(
            http_client.clone(),
            effective_order,
            config.registry.baller_registry_url.clone(),
            config.registry.chocolatey_feed_url.clone(),
        );

        Ok(Self {
            config,
            db,
            http_client,
            registry,
            downloader,
        })
    }
}
