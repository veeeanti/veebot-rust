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
    pub fn from_env() -> Result<Self, config::ConfigError> {
        // Try to find .env file in current directory
        let env_loaded = dotenv::dotenv().is_ok();
        tracing::info!("dotenv loaded: {}", env_loaded);
        
        // Debug: print current working directory and check if .env was loaded
        tracing::info!("Current dir: {:?}", std::env::current_dir());
        
        // Try to read .env directly to verify it's accessible
        if let Ok(contents) = std::fs::read_to_string(".env") {
            let has_openrouter = contents.contains("OPENROUTER_API_KEY");
            let has_ai_model = contents.contains("AI_MODEL");
            tracing::info!(".env file has OPENROUTER_API_KEY: {}, AI_MODEL: {}", has_openrouter, has_ai_model);
        }
        
        // Now read the env vars - this is the real test
        let openrouter_check = env::var("OPENROUTER_API_KEY");
        let ai_model_check = env::var("AI_MODEL");
        tracing::info!("OPENROUTER_API_KEY env::var result: {:?}", openrouter_check.is_ok());
        tracing::info!("AI_MODEL env::var result: {:?}", ai_model_check.is_ok());
        
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
