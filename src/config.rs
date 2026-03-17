//! Configuration module for veebot
//! Handles environment variables and configuration

use once_cell::sync::Lazy;
use serde::Deserialize;
use std::env;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    // Discord
    pub discord_token: String,
    pub guild_id: Option<String>,
    pub channel_id: Option<String>,
    
    // Bot behavior
    pub local: bool,
    pub ai_model: Option<String>,
    pub openrouter_api_key: Option<String>,
    pub random_response_chance: f64,
    pub prompt: String,
    pub debug: bool,
    pub enable_mentions: bool,
    pub enable_semantic_search: bool,
    pub enable_database: bool,
    pub database_type: DatabaseType,
    pub database_url: Option<String>,
    pub friendly_fire: bool,
    
    // Database
    pub sqlite_path: String,
    pub postgres_host: String,
    pub postgres_port: u16,
    pub postgres_db: String,
    pub postgres_user: String,
    pub postgres_password: String,
    pub postgres_ssl: bool,
    
    // Search
    pub search_engine: String,
    
    // Context
    pub max_context_messages: usize,
    pub context_similarity_threshold: f64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseType {
    Sqlite,
    Postgres,
}

impl Default for DatabaseType {
    fn default() -> Self {
        DatabaseType::Sqlite
    }
}

impl std::str::FromStr for DatabaseType {
    type Err = String;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "postgres" | "postgresql" => Ok(DatabaseType::Postgres),
            "sqlite" | _ => Ok(DatabaseType::Sqlite),
        }
    }
}

impl std::fmt::Display for DatabaseType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseType::Sqlite => f.write_str("sqlite"),
            DatabaseType::Postgres => f.write_str("postgres"),
        }
    }
}

impl Config {
    /// Find the project root by looking for Cargo.toml
    fn find_project_root() -> Option<std::path::PathBuf> {
        let mut path = std::env::current_dir().ok()?;
        
        // Look upwards for Cargo.toml
        loop {
            if path.join("Cargo.toml").exists() {
                return Some(path);
            }
            if !path.pop() {
                break;
            }
        }
        None
    }

    /// Load .env file manually from the given path
    fn load_env_file(path: &std::path::Path) -> Result<(), std::io::Error> {
        let contents = std::fs::read_to_string(path)?;
        for line in contents.lines() {
            let line = line.trim();
            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Parse KEY=VALUE format
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                // Remove surrounding quotes if present
                let value = value.trim_matches('"').trim_matches('\'');
                std::env::set_var(key, value);
            }
        }
        Ok(())
    }

    pub fn from_env() -> Result<Self, config::ConfigError> {
        // Try to find project root and load .env from there
        let project_root = Self::find_project_root();
        let env_path = project_root.as_ref().map(|p| p.join(".env"));
        
        let env_loaded = if let Some(ref path) = env_path {
            Self::load_env_file(path).is_ok()
        } else {
            dotenv::dotenv().is_ok()
        };
        
        if let Some(ref path) = env_path {
            tracing::info!("Loading .env from: {:?}", path);
        }
        tracing::info!("dotenv loaded: {}", env_loaded);
        
        let database_type_str = env::var("DATABASE_TYPE").unwrap_or_else(|_| "sqlite".to_string());
        let database_type: DatabaseType = database_type_str.parse().unwrap_or_default();
        
        let openrouter_api_key = env::var("OPENROUTER_API_KEY").ok();
        let ai_model = env::var("AI_MODEL").ok();
        
        // Debug logging for AI config
        if openrouter_api_key.is_some() {
            tracing::info!("OPENROUTER_API_KEY loaded: present");
        } else {
            tracing::warn!("OPENROUTER_API_KEY is missing or empty!");
        }
        if ai_model.is_some() {
            tracing::info!("AI_MODEL loaded: {}", ai_model.as_ref().unwrap());
        } else {
            tracing::warn!("AI_MODEL is missing or empty!");
        }
        
        Ok(Config {
            discord_token: env::var("DISCORD_TOKEN").unwrap_or_default(),
            guild_id: env::var("GUILD_ID").ok(),
            channel_id: env::var("CHANNEL_ID").ok(),
            local: env::var("LOCAL").map(|v| v == "true").unwrap_or(false),
            ai_model,
            openrouter_api_key,
            random_response_chance: env::var("RANDOM_RESPONSE_CHANCE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.1),
            prompt: env::var("PROMPT").unwrap_or_default(),
            debug: env::var("DEBUG").map(|v| v == "true").unwrap_or(false),
            enable_mentions: env::var("ENABLE_MENTIONS").map(|v| v == "true").unwrap_or(false),
            enable_semantic_search: env::var("ENABLE_SEMANTIC_SEARCH").map(|v| v == "true").unwrap_or(true),
            enable_database: env::var("ENABLE_DATABASE").map(|v| v == "true").unwrap_or(true),
            database_type,
            database_url: env::var("DATABASE_URL").ok(),
            friendly_fire: env::var("FRIENDLY_FIRE").map(|v| v == "true").unwrap_or(false),
            sqlite_path: env::var("SQLITE_PATH").unwrap_or_else(|_| "./database.sqlite".to_string()),
            postgres_host: env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string()),
            postgres_port: env::var("POSTGRES_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5432),
            postgres_db: env::var("POSTGRES_DB").unwrap_or_else(|_| "uc_aiv2".to_string()),
            postgres_user: env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string()),
            postgres_password: env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "password".to_string()),
            postgres_ssl: env::var("POSTGRES_SSL").map(|v| v == "true").unwrap_or(false),
            search_engine: env::var("SEARCH_ENGINE").unwrap_or_else(|_| "https://www.google.com/search?q=".to_string()),
            max_context_messages: env::var("MAX_CONTEXT_MESSAGES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20),
            context_similarity_threshold: env::var("CONTEXT_SIMILARITY_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.7),
        })
    }
}

/// Global configuration instance
pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    Config::from_env().expect("Failed to load configuration")
});

pub mod config {
    use thiserror::Error;
    
    #[derive(Error, Debug)]
    pub enum ConfigError {
        #[error("Environment variable error: {0}")]
        Env(#[from] std::env::VarError),
        #[error("Parse error: {0}")]
        Parse(String),
    }
}
