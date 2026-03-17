//! SQLite database implementation

use crate::database::{
    Birthday, Database, DatabaseError, DbResult, Memory, Message, MessageType, ServerSetting,
    StoreMessageData,
};
use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use std::path::Path;

const SCHEMA: &str = include_str!("../schema-sqlite.sql");

pub struct SqliteDatabase {
    pool: sqlx::SqlitePool,
}

impl SqliteDatabase {
    pub async fn new(path: &str) -> DbResult<Self> {
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DatabaseError::Connection(e.to_string()))?;
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(|e| DatabaseError::Connection(e.to_string()))?;

        Ok(Self { pool })
    }

    async fn execute_schema(&self) -> DbResult<()> {
        for statement in SCHEMA.split(';') {
            let stmt = statement.trim();
            if !stmt.is_empty() && !stmt.starts_with("--") {
                if let Err(e) = sqlx::query(stmt).execute(&self.pool).await {
                    if !e.to_string().contains("already exists") {
                        tracing::warn!("Schema statement error (ignoring): {}", e);
                    }
                }
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Database for SqliteDatabase {
    async fn test_connection(&self) -> DbResult<bool> {
        sqlx::query("SELECT datetime('now')")
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::Connection(e.to_string()))?;
        Ok(true)
    }

    async fn initialize(&self) -> DbResult<()> {
        self.execute_schema().await
    }

    async fn store_message(&self, data: StoreMessageData) -> DbResult<Option<Message>> {
        let message_type_str = match data.message_type {
            MessageType::User => "user",
            MessageType::Assistant => "assistant",
        };

        let row = sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "INSERT OR IGNORE INTO messages (discord_message_id, content, author_id, author_name, channel_id, guild_id, message_type)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             RETURNING id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at"
        )
        .bind(&data.discord_message_id)
        .bind(&data.content)
        .bind(&data.author_id)
        .bind(&data.author_name)
        .bind(&data.channel_id)
        .bind(&data.guild_id)
        .bind(message_type_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(row.map(message_from_row))
    }

    async fn find_similar_messages(&self, query: &str, guild_id: Option<&str>, author_id: Option<&str>, limit: usize) -> DbResult<Vec<Message>> {
        let rows = match (guild_id, author_id) {
            (Some(guild_id), Some(author_id)) => {
                sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT m.id, m.discord_message_id, m.content, m.author_id, m.author_name, m.message_type, m.guild_id, m.channel_id, m.created_at, m.updated_at
                     FROM messages m
                     JOIN messages_fts ON messages_fts.rowid = m.id
                     WHERE messages_fts MATCH ? AND m.guild_id = ? AND m.author_id = ?
                     ORDER BY m.created_at DESC
                     LIMIT ?"
                )
                .bind(query)
                .bind(guild_id)
                .bind(author_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
            (Some(guild_id), None) => {
                sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT m.id, m.discord_message_id, m.content, m.author_id, m.author_name, m.message_type, m.guild_id, m.channel_id, m.created_at, m.updated_at
                     FROM messages m
                     JOIN messages_fts ON messages_fts.rowid = m.id
                     WHERE messages_fts MATCH ? AND m.guild_id = ?
                     ORDER BY m.created_at DESC
                     LIMIT ?"
                )
                .bind(query)
                .bind(guild_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
            (None, Some(author_id)) => {
                sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT m.id, m.discord_message_id, m.content, m.author_id, m.author_name, m.message_type, m.guild_id, m.channel_id, m.created_at, m.updated_at
                     FROM messages m
                     JOIN messages_fts ON messages_fts.rowid = m.id
                     WHERE messages_fts MATCH ? AND m.author_id = ?
                     ORDER BY m.created_at DESC
                     LIMIT ?"
                )
                .bind(query)
                .bind(author_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
            (None, None) => {
                sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT m.id, m.discord_message_id, m.content, m.author_id, m.author_name, m.message_type, m.guild_id, m.channel_id, m.created_at, m.updated_at
                     FROM messages m
                     JOIN messages_fts ON messages_fts.rowid = m.id
                     WHERE messages_fts MATCH ?
                     ORDER BY m.created_at DESC
                     LIMIT ?"
                )
                .bind(query)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(message_from_row).collect())
    }

    async fn get_recent_messages(&self, guild_id: Option<&str>, author_id: Option<&str>, limit: usize) -> DbResult<Vec<Message>> {
        let rows = match (guild_id, author_id) {
            (Some(guild_id), Some(author_id)) => {
                sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at
                     FROM messages
                     WHERE guild_id = ? AND author_id = ?
                     ORDER BY created_at DESC
                     LIMIT ?"
                )
                .bind(guild_id)
                .bind(author_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
            (Some(guild_id), None) => {
                sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at
                     FROM messages
                     WHERE guild_id = ?
                     ORDER BY created_at DESC
                     LIMIT ?"
                )
                .bind(guild_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
            (None, Some(author_id)) => {
                sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at
                     FROM messages
                     WHERE author_id = ?
                     ORDER BY created_at DESC
                     LIMIT ?"
                )
                .bind(author_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
            (None, None) => {
                sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at
                     FROM messages
                     ORDER BY created_at DESC
                     LIMIT ?"
                )
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(message_from_row).collect())
    }

    async fn get_channel_messages(&self, channel_id: &str, limit: usize) -> DbResult<Vec<Message>> {
        let rows = sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "SELECT id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at
             FROM messages
             WHERE channel_id = ?
             ORDER BY created_at DESC
             LIMIT ?"
        )
        .bind(channel_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(message_from_row).collect())
    }

    async fn cleanup_old_messages(&self, days_old: i32) -> DbResult<u64> {
        let result = sqlx::query(
            "DELETE FROM messages WHERE created_at < datetime('now', ?) AND message_type = 'user'",
        )
        .bind(format!("-{} days", days_old))
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(result.rows_affected())
    }

    async fn store_memory(&self, user_id: &str, username: &str, memory: &str, guild_id: Option<&str>) -> DbResult<Option<Memory>> {
        let row = sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "INSERT INTO memories (user_id, username, memory, guild_id)
             VALUES (?, ?, ?, ?)
             RETURNING id, user_id, username, memory, guild_id, channel_id, created_at, updated_at"
        )
        .bind(user_id)
        .bind(username)
        .bind(memory)
        .bind(guild_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(row.map(memory_from_row))
    }

    async fn get_memories(&self, user_id: &str, guild_id: Option<&str>, limit: usize) -> DbResult<Vec<Memory>> {
        let rows = match guild_id {
            Some(guild_id) => {
                sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT id, user_id, username, memory, guild_id, channel_id, created_at, updated_at
                     FROM memories
                     WHERE user_id = ? AND guild_id = ?
                     ORDER BY created_at DESC
                     LIMIT ?"
                )
                .bind(user_id)
                .bind(guild_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT id, user_id, username, memory, guild_id, channel_id, created_at, updated_at
                     FROM memories
                     WHERE user_id = ?
                     ORDER BY created_at DESC
                     LIMIT ?"
                )
                .bind(user_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(memory_from_row).collect())
    }

    async fn search_memories(&self, query: &str, user_id: Option<&str>, guild_id: Option<&str>, limit: usize) -> DbResult<Vec<Memory>> {
        let rows = match (user_id, guild_id) {
            (Some(user_id), Some(guild_id)) => {
                sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT m.id, m.user_id, m.username, m.memory, m.guild_id, m.channel_id, m.created_at, m.updated_at
                     FROM memories m
                     JOIN memories_fts ON memories_fts.rowid = m.id
                     WHERE memories_fts MATCH ? AND m.user_id = ? AND m.guild_id = ?
                     ORDER BY m.created_at DESC
                     LIMIT ?"
                )
                .bind(query)
                .bind(user_id)
                .bind(guild_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
            (Some(user_id), None) => {
                sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT m.id, m.user_id, m.username, m.memory, m.guild_id, m.channel_id, m.created_at, m.updated_at
                     FROM memories m
                     JOIN memories_fts ON memories_fts.rowid = m.id
                     WHERE memories_fts MATCH ? AND m.user_id = ?
                     ORDER BY m.created_at DESC
                     LIMIT ?"
                )
                .bind(query)
                .bind(user_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
            (None, Some(guild_id)) => {
                sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT m.id, m.user_id, m.username, m.memory, m.guild_id, m.channel_id, m.created_at, m.updated_at
                     FROM memories m
                     JOIN memories_fts ON memories_fts.rowid = m.id
                     WHERE memories_fts MATCH ? AND m.guild_id = ?
                     ORDER BY m.created_at DESC
                     LIMIT ?"
                )
                .bind(query)
                .bind(guild_id)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
            (None, None) => {
                sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
                    "SELECT m.id, m.user_id, m.username, m.memory, m.guild_id, m.channel_id, m.created_at, m.updated_at
                     FROM memories m
                     JOIN memories_fts ON memories_fts.rowid = m.id
                     WHERE memories_fts MATCH ?
                     ORDER BY m.created_at DESC
                     LIMIT ?"
                )
                .bind(query)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(memory_from_row).collect())
    }

    async fn remove_memory(&self, memory_id: i64, user_id: &str) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM memories WHERE id = ? AND user_id = ?")
            .bind(memory_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn set_birthday(&self, user_id: &str, username: &str, day: i32, month: i32, year: Option<i32>) -> DbResult<bool> {
        let result = sqlx::query(
            "INSERT INTO birthdays (user_id, username, day, month, year)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(user_id) DO UPDATE SET username = excluded.username, day = excluded.day, month = excluded.month, year = excluded.year, updated_at = CURRENT_TIMESTAMP"
        )
        .bind(user_id)
        .bind(username)
        .bind(day)
        .bind(month)
        .bind(year)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_birthday(&self, user_id: &str) -> DbResult<Option<Birthday>> {
        let row = sqlx::query_as::<_, (String, String, i32, i32, Option<i32>, i32, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "SELECT user_id, username, day, month, year, last_pinged_year, created_at, updated_at FROM birthdays WHERE user_id = ?"
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(row.map(birthday_from_row))
    }

    async fn remove_birthday(&self, user_id: &str) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM birthdays WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_todays_birthdays(&self, day: i32, month: i32, current_year: i32) -> DbResult<Vec<Birthday>> {
        let rows = sqlx::query_as::<_, (String, String, i32, i32, Option<i32>, i32, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "SELECT user_id, username, day, month, year, last_pinged_year, created_at, updated_at FROM birthdays WHERE day = ? AND month = ? AND last_pinged_year < ?"
        )
        .bind(day)
        .bind(month)
        .bind(current_year)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(birthday_from_row).collect())
    }

    async fn mark_birthday_as_pinged(&self, user_id: &str, year: i32) -> DbResult<bool> {
        let result = sqlx::query("UPDATE birthdays SET last_pinged_year = ? WHERE user_id = ?")
            .bind(year)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn set_birthday_channel(&self, guild_id: &str, channel_id: &str) -> DbResult<bool> {
        let result = sqlx::query(
            "INSERT INTO birthday_channels (guild_id, channel_id)
             VALUES (?, ?)
             ON CONFLICT(guild_id) DO UPDATE SET channel_id = excluded.channel_id, updated_at = CURRENT_TIMESTAMP"
        )
        .bind(guild_id)
        .bind(channel_id)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_birthday_channel(&self, guild_id: &str) -> DbResult<Option<String>> {
        let row = sqlx::query_scalar::<_, String>("SELECT channel_id FROM birthday_channels WHERE guild_id = ?")
            .bind(guild_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(row)
    }

    async fn remove_birthday_channel(&self, guild_id: &str) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM birthday_channels WHERE guild_id = ?")
            .bind(guild_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn set_server_setting(&self, guild_id: &str, setting_name: &str, setting_value: &str) -> DbResult<bool> {
        let result = sqlx::query(
            "INSERT INTO server_settings (guild_id, setting_name, setting_value)
             VALUES (?, ?, ?)
             ON CONFLICT(guild_id, setting_name) DO UPDATE SET setting_value = excluded.setting_value, updated_at = CURRENT_TIMESTAMP"
        )
        .bind(guild_id)
        .bind(setting_name)
        .bind(setting_value)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_server_setting(&self, guild_id: &str, setting_name: &str) -> DbResult<Option<String>> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT setting_value FROM server_settings WHERE guild_id = ? AND setting_name = ?"
        )
        .bind(guild_id)
        .bind(setting_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(row)
    }

    async fn get_all_server_settings(&self, guild_id: &str) -> DbResult<Vec<ServerSetting>> {
        let rows = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT guild_id, setting_name, setting_value FROM server_settings WHERE guild_id = ?"
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| ServerSetting {
                guild_id: row.0,
                setting_name: row.1,
                setting_value: row.2,
            })
            .collect())
    }

    async fn remove_server_setting(&self, guild_id: &str, setting_name: &str) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM server_settings WHERE guild_id = ? AND setting_name = ?")
            .bind(guild_id)
            .bind(setting_name)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_message_count(&self) -> DbResult<i64> {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))
    }

    async fn get_unique_channel_count(&self) -> DbResult<i64> {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT channel_id) FROM messages")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))
    }
}

fn message_from_row(
    row: (
        i64,
        Option<String>,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ),
) -> Message {
    Message {
        id: row.0,
        discord_message_id: row.1,
        content: row.2,
        author_id: row.3,
        author_name: row.4,
        message_type: match row.5.as_str() {
            "assistant" => MessageType::Assistant,
            _ => MessageType::User,
        },
        guild_id: row.6,
        channel_id: row.7,
        created_at: row.8,
        updated_at: row.9,
    }
}

fn memory_from_row(
    row: (
        i64,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ),
) -> Memory {
    Memory {
        id: row.0,
        user_id: row.1,
        username: row.2,
        memory: row.3,
        guild_id: row.4,
        channel_id: row.5,
        created_at: row.6,
        updated_at: row.7,
    }
}

fn birthday_from_row(
    row: (
        String,
        String,
        i32,
        i32,
        Option<i32>,
        i32,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ),
) -> Birthday {
    Birthday {
        user_id: row.0,
        username: row.1,
        day: row.2,
        month: row.3,
        year: row.4,
        last_pinged_year: row.5,
        created_at: row.6,
        updated_at: row.7,
    }
}
