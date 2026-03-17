//! Database module for veebot
//! Handles SQLite and PostgreSQL connections and operations

pub use crate::sqlite::SqliteDatabase;
pub use crate::postgres::PostgresDatabase;

use crate::config::{Config, DatabaseType};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("Database connection error: {0}")]
    Connection(String),
    #[error("Query error: {0}")]
    Query(String),
    #[error("Not found")]
    NotFound,
}

pub type DbResult<T> = Result<T, DatabaseError>;

/// Message types for stored messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageType {
    User,
    Assistant,
}

/// Stored message structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub discord_message_id: Option<String>,
    pub content: String,
    pub author_id: String,
    pub author_name: String,
    pub message_type: MessageType,
    pub guild_id: Option<String>,
    pub channel_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Birthday information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Birthday {
    pub user_id: String,
    pub username: String,
    pub day: i32,
    pub month: i32,
    pub year: Option<i32>,
    pub last_pinged_year: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Memory stored for users
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub user_id: String,
    pub username: String,
    pub memory: String,
    pub guild_id: Option<String>,
    pub channel_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Server setting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSetting {
    pub guild_id: String,
    pub setting_name: String,
    pub setting_value: Option<String>,
}

/// Birthday channel for a guild
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BirthdayChannel {
    pub guild_id: String,
    pub channel_id: String,
}

/// Database trait for abstraction
#[async_trait]
pub trait Database: Send + Sync {
    // Connection
    async fn test_connection(&self) -> DbResult<bool>;
    async fn initialize(&self) -> DbResult<()>;
    
    // Messages
    async fn store_message(&self, data: StoreMessageData) -> DbResult<Option<Message>>;
    async fn find_similar_messages(&self, query: &str, guild_id: Option<&str>, author_id: Option<&str>, limit: usize) -> DbResult<Vec<Message>>;
    async fn get_recent_messages(&self, guild_id: Option<&str>, author_id: Option<&str>, limit: usize) -> DbResult<Vec<Message>>;
    async fn get_channel_messages(&self, channel_id: &str, limit: usize) -> DbResult<Vec<Message>>;
    async fn cleanup_old_messages(&self, days_old: i32) -> DbResult<u64>;
    
    // Memories
    async fn store_memory(&self, user_id: &str, username: &str, memory: &str, guild_id: Option<&str>) -> DbResult<Option<Memory>>;
    async fn get_memories(&self, user_id: &str, guild_id: Option<&str>, limit: usize) -> DbResult<Vec<Memory>>;
    async fn search_memories(&self, query: &str, user_id: Option<&str>, guild_id: Option<&str>, limit: usize) -> DbResult<Vec<Memory>>;
    async fn remove_memory(&self, memory_id: i64, user_id: &str) -> DbResult<bool>;
    
    // Birthdays
    async fn set_birthday(&self, user_id: &str, username: &str, day: i32, month: i32, year: Option<i32>) -> DbResult<bool>;
    async fn get_birthday(&self, user_id: &str) -> DbResult<Option<Birthday>>;
    async fn remove_birthday(&self, user_id: &str) -> DbResult<bool>;
    async fn get_todays_birthdays(&self, day: i32, month: i32, current_year: i32) -> DbResult<Vec<Birthday>>;
    async fn mark_birthday_as_pinged(&self, user_id: &str, year: i32) -> DbResult<bool>;
    async fn set_birthday_channel(&self, guild_id: &str, channel_id: &str) -> DbResult<bool>;
    async fn get_birthday_channel(&self, guild_id: &str) -> DbResult<Option<String>>;
    async fn remove_birthday_channel(&self, guild_id: &str) -> DbResult<bool>;
    
    // Server settings
    async fn set_server_setting(&self, guild_id: &str, setting_name: &str, setting_value: &str) -> DbResult<bool>;
    async fn get_server_setting(&self, guild_id: &str, setting_name: &str) -> DbResult<Option<String>>;
    async fn get_all_server_settings(&self, guild_id: &str) -> DbResult<Vec<ServerSetting>>;
    async fn remove_server_setting(&self, guild_id: &str, setting_name: &str) -> DbResult<bool>;
    
    // Statistics
    async fn get_message_count(&self) -> DbResult<i64>;
    async fn get_unique_channel_count(&self) -> DbResult<i64>;
}

/// Data for storing a message
#[derive(Debug, Clone)]
pub struct StoreMessageData {
    pub discord_message_id: String,
    pub content: String,
    pub author_id: String,
    pub author_name: String,
    pub channel_id: String,
    pub guild_id: Option<String>,
    pub message_type: MessageType,
}

/// Database wrapper that holds either SQLite or PostgreSQL
pub struct DatabaseManager {
    db: Arc<RwLock<Box<dyn Database>>>,
}

impl DatabaseManager {
    pub async fn new(config: &Config) -> DbResult<Self> {
        let db: Box<dyn Database> = match config.database_type {
            DatabaseType::Sqlite => Box::new(SqliteDatabase::new(&config.sqlite_path).await?),
            DatabaseType::Postgres => {
                let url = config.database_url.clone().unwrap_or_else(|| {
                    format!(
                        "postgres://{}:{}@{}:{}/{}",
                        config.postgres_user,
                        config.postgres_password,
                        config.postgres_host,
                        config.postgres_port,
                        config.postgres_db
                    )
                });
                Box::new(PostgresDatabase::new(&url).await?)
            }
        };
        
        Ok(Self {
            db: Arc::new(RwLock::new(db)),
        })
    }
    
    pub async fn test_connection(&self) -> DbResult<bool> {
        self.db.read().await.test_connection().await
    }
    
    pub async fn initialize(&self) -> DbResult<()> {
        self.db.read().await.initialize().await
    }
    
    pub async fn store_message(&self, data: StoreMessageData) -> DbResult<Option<Message>> {
        self.db.read().await.store_message(data).await
    }
    
    pub async fn find_similar_messages(&self, query: &str, guild_id: Option<&str>, author_id: Option<&str>, limit: usize) -> DbResult<Vec<Message>> {
        self.db.read().await.find_similar_messages(query, guild_id, author_id, limit).await
    }
    
    pub async fn get_recent_messages(&self, guild_id: Option<&str>, author_id: Option<&str>, limit: usize) -> DbResult<Vec<Message>> {
        self.db.read().await.get_recent_messages(guild_id, author_id, limit).await
    }
    
    pub async fn get_channel_messages(&self, channel_id: &str, limit: usize) -> DbResult<Vec<Message>> {
        self.db.read().await.get_channel_messages(channel_id, limit).await
    }
    
    pub async fn cleanup_old_messages(&self, days_old: i32) -> DbResult<u64> {
        self.db.read().await.cleanup_old_messages(days_old).await
    }
    
    pub async fn store_memory(&self, user_id: &str, username: &str, memory: &str, guild_id: Option<&str>) -> DbResult<Option<Memory>> {
        self.db.read().await.store_memory(user_id, username, memory, guild_id).await
    }
    
    pub async fn get_memories(&self, user_id: &str, guild_id: Option<&str>, limit: usize) -> DbResult<Vec<Memory>> {
        self.db.read().await.get_memories(user_id, guild_id, limit).await
    }
    
    pub async fn search_memories(&self, query: &str, user_id: Option<&str>, guild_id: Option<&str>, limit: usize) -> DbResult<Vec<Memory>> {
        self.db.read().await.search_memories(query, user_id, guild_id, limit).await
    }
    
    pub async fn remove_memory(&self, memory_id: i64, user_id: &str) -> DbResult<bool> {
        self.db.read().await.remove_memory(memory_id, user_id).await
    }
    
    pub async fn set_birthday(&self, user_id: &str, username: &str, day: i32, month: i32, year: Option<i32>) -> DbResult<bool> {
        self.db.read().await.set_birthday(user_id, username, day, month, year).await
    }
    
    pub async fn get_birthday(&self, user_id: &str) -> DbResult<Option<Birthday>> {
        self.db.read().await.get_birthday(user_id).await
    }
    
    pub async fn remove_birthday(&self, user_id: &str) -> DbResult<bool> {
        self.db.read().await.remove_birthday(user_id).await
    }
    
    pub async fn get_todays_birthdays(&self, day: i32, month: i32, current_year: i32) -> DbResult<Vec<Birthday>> {
        self.db.read().await.get_todays_birthdays(day, month, current_year).await
    }
    
    pub async fn mark_birthday_as_pinged(&self, user_id: &str, year: i32) -> DbResult<bool> {
        self.db.read().await.mark_birthday_as_pinged(user_id, year).await
    }
    
    pub async fn set_birthday_channel(&self, guild_id: &str, channel_id: &str) -> DbResult<bool> {
        self.db.read().await.set_birthday_channel(guild_id, channel_id).await
    }
    
    pub async fn get_birthday_channel(&self, guild_id: &str) -> DbResult<Option<String>> {
        self.db.read().await.get_birthday_channel(guild_id).await
    }
    
    pub async fn remove_birthday_channel(&self, guild_id: &str) -> DbResult<bool> {
        self.db.read().await.remove_birthday_channel(guild_id).await
    }
    
    pub async fn set_server_setting(&self, guild_id: &str, setting_name: &str, setting_value: &str) -> DbResult<bool> {
        self.db.read().await.set_server_setting(guild_id, setting_name, setting_value).await
    }
    
    pub async fn get_server_setting(&self, guild_id: &str, setting_name: &str) -> DbResult<Option<String>> {
        self.db.read().await.get_server_setting(guild_id, setting_name).await
    }
    
    pub async fn get_all_server_settings(&self, guild_id: &str) -> DbResult<Vec<ServerSetting>> {
        self.db.read().await.get_all_server_settings(guild_id).await
    }
    
    pub async fn remove_server_setting(&self, guild_id: &str, setting_name: &str) -> DbResult<bool> {
        self.db.read().await.remove_server_setting(guild_id, setting_name).await
    }
    
    pub async fn get_message_count(&self) -> DbResult<i64> {
        self.db.read().await.get_message_count().await
    }
    
    pub async fn get_unique_channel_count(&self) -> DbResult<i64> {
        self.db.read().await.get_unique_channel_count().await
    }
}
