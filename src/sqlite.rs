//! SQLite database implementation

use super::{Database, DatabaseError, DbResult, Message, MessageType, Memory, Birthday, ServerSetting, StoreMessageData};
use async_trait::async_trait;
use rusqlite::{params, Connection};
use std::path::Path;
use tokio::task;

const SCHEMA: &str = include_str!("../schema-sqlite.sql");

pub struct SqliteDatabase {
    conn: Connection,
}

impl SqliteDatabase {
    pub async fn new(path: &str) -> DbResult<Self> {
        // Ensure parent directory exists
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| DatabaseError::Connection(e.to_string()))?;
        }
        
        let conn = Connection::open(path)
            .map_err(|e| DatabaseError::Connection(e.to_string()))?;
        
        // Enable WAL mode for better concurrency
        conn.execute_batch("PRAGMA journal_mode = WAL;")
            .map_err(|e| DatabaseError::Connection(e.to_string()))?;
        
        Ok(Self { conn })
    }
    
    fn execute_schema(&self) -> DbResult<()> {
        // Split by semicolons and execute each statement
        for statement in SCHEMA.split(';') {
            let stmt = statement.trim();
            if !stmt.is_empty() && !stmt.starts_with("--") {
                if let Err(e) = self.conn.execute(stmt, []) {
                    // Ignore "already exists" errors
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
        task::spawn_blocking(|| {
            self.conn.query_row(
                "SELECT datetime('now') as current_time",
                [],
                |_| Ok(true)
            )
        })
        .await
        .map_err(|e| DatabaseError::Connection(e.to_string()))?
        .map_err(|e| DatabaseError::Connection(e.to_string()))
    }
    
    async fn initialize(&self) -> DbResult<()> {
        task::spawn_blocking(|| {
            self.execute_schema()
        })
        .await
        .map_err(|e| DatabaseError::Connection(e.to_string()))?
    }
    
    async fn store_message(&self, data: StoreMessageData) -> DbResult<Option<Message>> {
        let discord_message_id = data.discord_message_id.clone();
        let content = data.content.clone();
        let author_id = data.author_id.clone();
        let author_name = data.author_name.clone();
        let channel_id = data.channel_id.clone();
        let guild_id = data.guild_id.clone();
        let message_type = data.message_type.clone();
        
        task::spawn_blocking(move || {
            let message_type_str = match message_type {
                MessageType::User => "user",
                MessageType::Assistant => "assistant",
            };
            
            let result = self.conn.execute(
                "INSERT OR IGNORE INTO messages (discord_message_id, content, author_id, author_name, channel_id, guild_id, message_type) VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![discord_message_id, content, author_id, author_name, channel_id, guild_id, message_type_str],
            );
            
            match result {
                Ok(changes) if changes > 0 => {
                    let row = self.conn.query_row(
                        "SELECT id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at FROM messages WHERE discord_message_id = ?",
                        params![discord_message_id],
                        |row| {
                            Ok(Message {
                                id: row.get(0)?,
                                discord_message_id: row.get(1)?,
                                content: row.get(2)?,
                                author_id: row.get(3)?,
                                author_name: row.get(4)?,
                                message_type: match row.get::<_, String>(5)?.as_str() {
                                    "assistant" => MessageType::Assistant,
                                    _ => MessageType::User,
                                },
                                guild_id: row.get(6)?,
                                channel_id: row.get(7)?,
                                created_at: row.get(8)?,
                                updated_at: row.get(9)?,
                            })
                        },
                    );
                    Ok(Some(row?))
                }
                _ => Ok(None),
            }
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn find_similar_messages(&self, query: &str, guild_id: Option<&str>, author_id: Option<&str>, limit: usize) -> DbResult<Vec<Message>> {
        let query = query.to_string();
        let guild_id = guild_id.map(|s| s.to_string());
        let author_id = author_id.map(|s| s.to_string());
        
        task::spawn_blocking(move || {
            let mut sql = String::from(
                "SELECT m.id, m.discord_message_id, m.content, m.author_id, m.author_name, m.message_type, m.guild_id, m.channel_id, m.created_at, m.updated_at, rank as similarity_score
                 FROM messages m
                 JOIN messages_fts f ON m.id = f.rowid
                 WHERE messages_fts MATCH ?"
            );
            
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(query)];
            
            if guild_id.is_some() {
                sql.push_str(" AND m.guild_id = ?");
                params_vec.push(Box::new(guild_id.unwrap()));
            }
            if author_id.is_some() {
                sql.push_str(" AND m.author_id = ?");
                params_vec.push(Box::new(author_id.unwrap()));
            }
            
            sql.push_str(" ORDER BY rank LIMIT ?");
            params_vec.push(Box::new(limit as i64));
            
            let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
            
            let mut stmt = self.conn.prepare(&sql).map_err(|e| DatabaseError::Query(e.to_string()))?;
            let rows = stmt.query_map(params_refs.as_slice(), |row| {
                Ok(Message {
                    id: row.get(0)?,
                    discord_message_id: row.get(1)?,
                    content: row.get(2)?,
                    author_id: row.get(3)?,
                    author_name: row.get(4)?,
                    message_type: match row.get::<_, String>(5)?.as_str() {
                        "assistant" => MessageType::Assistant,
                        _ => MessageType::User,
                    },
                    guild_id: row.get(6)?,
                    channel_id: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            }).map_err(|e| DatabaseError::Query(e.to_string()))?;
            
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row.map_err(|e| DatabaseError::Query(e.to_string()))?);
            }
            Ok(messages)
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn get_recent_messages(&self, guild_id: Option<&str>, author_id: Option<&str>, limit: usize) -> DbResult<Vec<Message>> {
        let guild_id = guild_id.map(|s| s.to_string());
        let author_id = author_id.map(|s| s.to_string());
        
        task::spawn_blocking(move || {
            let mut sql = String::from("SELECT id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at FROM messages");
            let mut conditions = Vec::new();
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            
            if guild_id.is_some() {
                conditions.push("guild_id = ?");
                params_vec.push(Box::new(guild_id.unwrap()));
            }
            if author_id.is_some() {
                conditions.push("author_id = ?");
                params_vec.push(Box::new(author_id.unwrap()));
            }
            
            if !conditions.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&conditions.join(" AND "));
            }
            
            sql.push_str(" ORDER BY created_at DESC LIMIT ?");
            params_vec.push(Box::new(limit as i64));
            
            let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
            
            let mut stmt = self.conn.prepare(&sql).map_err(|e| DatabaseError::Query(e.to_string()))?;
            let rows = stmt.query_map(params_refs.as_slice(), |row| {
                Ok(Message {
                    id: row.get(0)?,
                    discord_message_id: row.get(1)?,
                    content: row.get(2)?,
                    author_id: row.get(3)?,
                    author_name: row.get(4)?,
                    message_type: match row.get::<_, String>(5)?.as_str() {
                        "assistant" => MessageType::Assistant,
                        _ => MessageType::User,
                    },
                    guild_id: row.get(6)?,
                    channel_id: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            }).map_err(|e| DatabaseError::Query(e.to_string()))?;
            
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row.map_err(|e| DatabaseError::Query(e.to_string()))?);
            }
            Ok(messages)
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn get_channel_messages(&self, channel_id: &str, limit: usize) -> DbResult<Vec<Message>> {
        let channel_id = channel_id.to_string();
        
        task::spawn_blocking(move || {
            let mut stmt = self.conn.prepare(
                "SELECT id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at
                 FROM messages
                 WHERE channel_id = ?
                 ORDER BY created_at DESC
                 LIMIT ?"
            ).map_err(|e| DatabaseError::Query(e.to_string()))?;
            
            let rows = stmt.query_map(params![channel_id, limit as i64], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    discord_message_id: row.get(1)?,
                    content: row.get(2)?,
                    author_id: row.get(3)?,
                    author_name: row.get(4)?,
                    message_type: match row.get::<_, String>(5)?.as_str() {
                        "assistant" => MessageType::Assistant,
                        _ => MessageType::User,
                    },
                    guild_id: row.get(6)?,
                    channel_id: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            }).map_err(|e| DatabaseError::Query(e.to_string()))?;
            
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row.map_err(|e| DatabaseError::Query(e.to_string()))?);
            }
            Ok(messages)
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn cleanup_old_messages(&self, days_old: i32) -> DbResult<u64> {
        task::spawn_blocking(move || {
            let result = self.conn.execute(
                "DELETE FROM messages WHERE created_at < datetime('now', ?) AND message_type = 'user'",
                params![format!("-{} days", days_old)],
            );
            Ok(result.map(|c| c as u64).unwrap_or(0))
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn store_memory(&self, user_id: &str, username: &str, memory: &str, guild_id: Option<&str>) -> DbResult<Option<Memory>> {
        let user_id = user_id.to_string();
        let username = username.to_string();
        let memory = memory.to_string();
        let guild_id = guild_id.map(|s| s.to_string());
        
        task::spawn_blocking(move || {
            let result = self.conn.execute(
                "INSERT INTO memories (user_id, username, memory, guild_id) VALUES (?, ?, ?, ?)",
                params![user_id, username, memory, guild_id],
            );
            
            match result {
                Ok(_) => {
                    let row = self.conn.query_row(
                        "SELECT id, user_id, username, memory, guild_id, channel_id, created_at, updated_at FROM memories WHERE user_id = ? ORDER BY id DESC LIMIT 1",
                        params![user_id],
                        |row| {
                            Ok(Memory {
                                id: row.get(0)?,
                                user_id: row.get(1)?,
                                username: row.get(2)?,
                                memory: row.get(3)?,
                                guild_id: row.get(4)?,
                                channel_id: row.get(5)?,
                                created_at: row.get(6)?,
                                updated_at: row.get(7)?,
                            })
                        },
                    );
                    Ok(Some(row?))
                }
                _ => Ok(None),
            }
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn get_memories(&self, user_id: &str, guild_id: Option<&str>, limit: usize) -> DbResult<Vec<Memory>> {
        let user_id = user_id.to_string();
        let guild_id = guild_id.map(|s| s.to_string());
        
        task::spawn_blocking(move || {
            let mut sql = String::from("SELECT id, user_id, username, memory, guild_id, channel_id, created_at, updated_at FROM memories WHERE user_id = ?");
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(user_id.clone())];
            
            if guild_id.is_some() {
                sql.push_str(" AND guild_id = ?");
                params_vec.push(Box::new(guild_id.unwrap()));
            }
            
            sql.push_str(" ORDER BY created_at DESC LIMIT ?");
            params_vec.push(Box::new(limit as i64));
            
            let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
            
            let mut stmt = self.conn.prepare(&sql).map_err(|e| DatabaseError::Query(e.to_string()))?;
            let rows = stmt.query_map(params_refs.as_slice(), |row| {
                Ok(Memory {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    username: row.get(2)?,
                    memory: row.get(3)?,
                    guild_id: row.get(4)?,
                    channel_id: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            }).map_err(|e| DatabaseError::Query(e.to_string()))?;
            
            let mut memories = Vec::new();
            for row in rows {
                memories.push(row.map_err(|e| DatabaseError::Query(e.to_string()))?);
            }
            Ok(memories)
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn search_memories(&self, query: &str, user_id: Option<&str>, guild_id: Option<&str>, limit: usize) -> DbResult<Vec<Memory>> {
        let query = query.to_string();
        let user_id = user_id.map(|s| s.to_string());
        let guild_id = guild_id.map(|s| s.to_string());
        
        task::spawn_blocking(move || {
            let mut sql = String::from(
                "SELECT m.id, m.user_id, m.username, m.memory, m.guild_id, m.channel_id, m.created_at, m.updated_at, rank as similarity_score
                 FROM memories m
                 JOIN memories_fts f ON m.id = f.rowid
                 WHERE memories_fts MATCH ?"
            );
            
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(query)];
            
            if user_id.is_some() {
                sql.push_str(" AND m.user_id = ?");
                params_vec.push(Box::new(user_id.unwrap()));
            }
            if guild_id.is_some() {
                sql.push_str(" AND m.guild_id = ?");
                params_vec.push(Box::new(guild_id.unwrap()));
            }
            
            sql.push_str(" ORDER BY rank LIMIT ?");
            params_vec.push(Box::new(limit as i64));
            
            let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
            
            let mut stmt = self.conn.prepare(&sql).map_err(|e| DatabaseError::Query(e.to_string()))?;
            let rows = stmt.query_map(params_refs.as_slice(), |row| {
                Ok(Memory {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    username: row.get(2)?,
                    memory: row.get(3)?,
                    guild_id: row.get(4)?,
                    channel_id: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            }).map_err(|e| DatabaseError::Query(e.to_string()))?;
            
            let mut memories = Vec::new();
            for row in rows {
                memories.push(row.map_err(|e| DatabaseError::Query(e.to_string()))?);
            }
            Ok(memories)
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn remove_memory(&self, memory_id: i64, user_id: &str) -> DbResult<bool> {
        let user_id = user_id.to_string();
        
        task::spawn_blocking(move || {
            let result = self.conn.execute(
                "DELETE FROM memories WHERE id = ? AND user_id = ?",
                params![memory_id, user_id],
            );
            Ok(result.map(|c| c > 0).unwrap_or(false))
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn set_birthday(&self, user_id: &str, username: &str, day: i32, month: i32, year: Option<i32>) -> DbResult<bool> {
        let user_id = user_id.to_string();
        let username = username.to_string();
        
        task::spawn_blocking(move || {
            self.conn.execute(
                "INSERT INTO birthdays (user_id, username, day, month, year) VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(user_id) DO UPDATE SET username = EXCLUDED.username, day = EXCLUDED.day, month = EXCLUDED.month, year = EXCLUDED.year, updated_at = CURRENT_TIMESTAMP",
                params![user_id, username, day, month, year],
            ).map(|c| c > 0).map_err(|e| DatabaseError::Query(e.to_string()))
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn get_birthday(&self, user_id: &str) -> DbResult<Option<Birthday>> {
        let user_id = user_id.to_string();
        
        task::spawn_blocking(move || {
            let row = self.conn.query_row(
                "SELECT user_id, username, day, month, year, last_pinged_year, created_at, updated_at FROM birthdays WHERE user_id = ?",
                params![user_id],
                |row| {
                    Ok(Birthday {
                        user_id: row.get(0)?,
                        username: row.get(1)?,
                        day: row.get(2)?,
                        month: row.get(3)?,
                        year: row.get(4)?,
                        last_pinged_year: row.get(5)?,
                        created_at: row.get(6)?,
                        updated_at: row.get(7)?,
                    })
                },
            );
            
            match row {
                Ok(b) => Ok(Some(b)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(DatabaseError::Query(e.to_string())),
            }
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn remove_birthday(&self, user_id: &str) -> DbResult<bool> {
        let user_id = user_id.to_string();
        
        task::spawn_blocking(move || {
            self.conn.execute(
                "DELETE FROM birthdays WHERE user_id = ?",
                params![user_id],
            ).map(|c| c > 0).map_err(|e| DatabaseError::Query(e.to_string()))
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn get_todays_birthdays(&self, day: i32, month: i32, current_year: i32) -> DbResult<Vec<Birthday>> {
        task::spawn_blocking(move || {
            let mut stmt = self.conn.prepare(
                "SELECT user_id, username, day, month, year, last_pinged_year, created_at, updated_at FROM birthdays WHERE day = ? AND month = ? AND last_pinged_year < ?"
            ).map_err(|e| DatabaseError::Query(e.to_string()))?;
            
            let rows = stmt.query_map(params![day, month, current_year], |row| {
                Ok(Birthday {
                    user_id: row.get(0)?,
                    username: row.get(1)?,
                    day: row.get(2)?,
                    month: row.get(3)?,
                    year: row.get(4)?,
                    last_pinged_year: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            }).map_err(|e| DatabaseError::Query(e.to_string()))?;
            
            let mut birthdays = Vec::new();
            for row in rows {
                birthdays.push(row.map_err(|e| DatabaseError::Query(e.to_string()))?);
            }
            Ok(birthdays)
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn mark_birthday_as_pinged(&self, user_id: &str, year: i32) -> DbResult<bool> {
        let user_id = user_id.to_string();
        
        task::spawn_blocking(move || {
            self.conn.execute(
                "UPDATE birthdays SET last_pinged_year = ? WHERE user_id = ?",
                params![year, user_id],
            ).map(|c| c > 0).map_err(|e| DatabaseError::Query(e.to_string()))
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn set_birthday_channel(&self, guild_id: &str, channel_id: &str) -> DbResult<bool> {
        let guild_id = guild_id.to_string();
        let channel_id = channel_id.to_string();
        
        task::spawn_blocking(move || {
            self.conn.execute(
                "INSERT INTO birthday_channels (guild_id, channel_id) VALUES (?, ?)
                 ON CONFLICT(guild_id) DO UPDATE SET channel_id = EXCLUDED.channel_id, updated_at = CURRENT_TIMESTAMP",
                params![guild_id, channel_id],
            ).map(|c| c > 0).map_err(|e| DatabaseError::Query(e.to_string()))
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn get_birthday_channel(&self, guild_id: &str) -> DbResult<Option<String>> {
        let guild_id = guild_id.to_string();
        
        task::spawn_blocking(move || {
            let row = self.conn.query_row(
                "SELECT channel_id FROM birthday_channels WHERE guild_id = ?",
                params![guild_id],
                |row| row.get(0),
            );
            
            match row {
                Ok(c) => Ok(Some(c)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(DatabaseError::Query(e.to_string())),
            }
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn remove_birthday_channel(&self, guild_id: &str) -> DbResult<bool> {
        let guild_id = guild_id.to_string();
        
        task::spawn_blocking(move || {
            self.conn.execute(
                "DELETE FROM birthday_channels WHERE guild_id = ?",
                params![guild_id],
            ).map(|c| c > 0).map_err(|e| DatabaseError::Query(e.to_string()))
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn set_server_setting(&self, guild_id: &str, setting_name: &str, setting_value: &str) -> DbResult<bool> {
        let guild_id = guild_id.to_string();
        let setting_name = setting_name.to_string();
        let setting_value = setting_value.to_string();
        
        task::spawn_blocking(move || {
            self.conn.execute(
                "INSERT INTO server_settings (guild_id, setting_name, setting_value) VALUES (?, ?, ?)
                 ON CONFLICT(guild_id, setting_name) DO UPDATE SET setting_value = EXCLUDED.setting_value, updated_at = CURRENT_TIMESTAMP",
                params![guild_id, setting_name, setting_value],
            ).map(|c| c > 0).map_err(|e| DatabaseError::Query(e.to_string()))
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn get_server_setting(&self, guild_id: &str, setting_name: &str) -> DbResult<Option<String>> {
        let guild_id = guild_id.to_string();
        let setting_name = setting_name.to_string();
        
        task::spawn_blocking(move || {
            let row = self.conn.query_row(
                "SELECT setting_value FROM server_settings WHERE guild_id = ? AND setting_name = ?",
                params![guild_id, setting_name],
                |row| row.get(0),
            );
            
            match row {
                Ok(v) => Ok(Some(v)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(DatabaseError::Query(e.to_string())),
            }
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn get_all_server_settings(&self, guild_id: &str) -> DbResult<Vec<ServerSetting>> {
        let guild_id = guild_id.to_string();
        
        task::spawn_blocking(move || {
            let mut stmt = self.conn.prepare(
                "SELECT guild_id, setting_name, setting_value FROM server_settings WHERE guild_id = ?"
            ).map_err(|e| DatabaseError::Query(e.to_string()))?;
            
            let rows = stmt.query_map(params![guild_id], |row| {
                Ok(ServerSetting {
                    guild_id: row.get(0)?,
                    setting_name: row.get(1)?,
                    setting_value: row.get(2)?,
                })
            }).map_err(|e| DatabaseError::Query(e.to_string()))?;
            
            let mut settings = Vec::new();
            for row in rows {
                settings.push(row.map_err(|e| DatabaseError::Query(e.to_string()))?);
            }
            Ok(settings)
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn remove_server_setting(&self, guild_id: &str, setting_name: &str) -> DbResult<bool> {
        let guild_id = guild_id.to_string();
        let setting_name = setting_name.to_string();
        
        task::spawn_blocking(move || {
            self.conn.execute(
                "DELETE FROM server_settings WHERE guild_id = ? AND setting_name = ?",
                params![guild_id, setting_name],
            ).map(|c| c > 0).map_err(|e| DatabaseError::Query(e.to_string()))
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn get_message_count(&self) -> DbResult<i64> {
        task::spawn_blocking(|| {
            self.conn.query_row(
                "SELECT COUNT(*) FROM messages",
                [],
                |row| row.get(0),
            ).map_err(|e| DatabaseError::Query(e.to_string()))
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
    
    async fn get_unique_channel_count(&self) -> DbResult<i64> {
        task::spawn_blocking(|| {
            self.conn.query_row(
                "SELECT COUNT(DISTINCT channel_id) FROM messages",
                [],
                |row| row.get(0),
            ).map_err(|e| DatabaseError::Query(e.to_string()))
        })
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
    }
}
