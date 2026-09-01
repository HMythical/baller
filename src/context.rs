use crate::config::config::{BallerConfig, RegistryConfig};
use crate::core::db::DbManager;
use crate::core::downloader::Downloader;
use crate::core::registry::{RegistryClient, RegistrySource};
use crate::error::error::BallError;
use crate::http::HttpClient;
use crate::utils::fs::ensure_dir;

/// Resolve the configured `source_order` into the chain the registry client
/// actually queries: entries whose source is disabled — and names that match no
/// known source — are dropped, and the configured order is preserved.
pub fn effective_source_order(registry: &RegistryConfig) -> Vec<RegistrySource> {
    registry
        .source_order
        .iter()
        .filter_map(|name| RegistrySource::from_config_name(name))
        .filter(|source| match source {
            RegistrySource::GitHub => registry.github_enabled,
            RegistrySource::BallerRegistry => registry.baller_enabled,
            RegistrySource::Chocolatey => registry.chocolatey_enabled,
            RegistrySource::System => registry.system_enabled,
            RegistrySource::Cargo => registry.cargo_enabled,
        })
        .collect()
}

/// Flags accepted by every subcommand, resolved once from the CLI.
#[derive(Debug, Clone, Default)]
pub struct GlobalFlags {
    pub yes: bool,
    pub quiet: bool,
    pub json: bool,
    pub verbose: u8,
}

impl GlobalFlags {
    /// Whether human-readable progress output should be withheld.
    ///
    /// `--json` implies quiet so stdout stays parseable.
    pub fn is_quiet(&self) -> bool {
        self.quiet || self.json
    }
}

pub struct AppContext {
    pub config: BallerConfig,
    pub db: DbManager,
    #[allow(dead_code)]
    pub http_client: HttpClient,
    pub registry: RegistryClient,
    pub downloader: Downloader,
    pub flags: GlobalFlags,
}

impl AppContext {
    pub fn new(config: BallerConfig, flags: GlobalFlags) -> Result<Self, BallError> {
        ensure_dir(&config.hooks_dir)?;
        ensure_dir(&config.cache_dir)?;

        let http_client = HttpClient::new()?;
        let db = DbManager::init_at_path(&config.db_path)?;
        let downloader = Downloader::new(config.cache_dir.clone(), http_client.clone());

        let effective_order = effective_source_order(&config.registry);

        let registry = RegistryClient::with_source_order(
            http_client.clone(),
            effective_order,
            config.registry.baller_registry_url.clone(),
            config.registry.chocolatey_feed_url.clone(),
            config.registry.github_default_owner.clone(),
        );

        Ok(Self {
            config,
            db,
            http_client,
            registry,
            downloader,
            flags,
        })
    }
}
