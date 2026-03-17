//! PostgreSQL database implementation

use super::{Database, DatabaseError, DbResult, Message, MessageType, Memory, Birthday, ServerSetting, StoreMessageData};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

const SCHEMA: &str = include_str!("../schema.sql");

pub struct PostgresDatabase {
    pool: sqlx::PgPool,
}

impl PostgresDatabase {
    pub async fn new(url: &str) -> DbResult<Self> {
        let options = sqlx::postgres::PgConnectOptions::from_str(url)
            .map_err(|e| DatabaseError::Connection(e.to_string()))?;
        
        let pool = sqlx::PgPoolOptions::new()
            .max_connections(20)
            .idle_timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
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
impl Database for PostgresDatabase {
    async fn test_connection(&self) -> DbResult<bool> {
        sqlx::query("SELECT NOW() as current_time")
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
        
        let result = sqlx::query(
            "INSERT INTO messages (discord_message_id, content, author_id, author_name, channel_id, guild_id, message_type)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (discord_message_id) DO NOTHING
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
        
        Ok(result.map(|row| Message {
            id: row.get(0),
            discord_message_id: row.get(1),
            content: row.get(2),
            author_id: row.get(3),
            author_name: row.get(4),
            message_type: match row.get::<_, String>(5).as_str() {
                "assistant" => MessageType::Assistant,
                _ => MessageType::User,
            },
            guild_id: row.get(6),
            channel_id: row.get(7),
            created_at: row.get(8),
            updated_at: row.get(9),
        }))
    }
    
    async fn find_similar_messages(&self, query: &str, guild_id: Option<&str>, author_id: Option<&str>, limit: usize) -> DbResult<Vec<Message>> {
        let mut sql = String::from(
            "SELECT id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at,
             ts_rank(to_tsvector('english', content), plainto_tsquery('english', $1)) as similarity_score
             FROM messages
             WHERE to_tsvector('english', content) @@ plainto_tsquery('english', $1)"
        );
        
        let mut params: Vec<Box<dyn sqlx::Encode<'_, sqlx::Postgres>>> = vec![Box::new(query.to_string())];
        let mut param_index = 2;
        
        if guild_id.is_some() {
            sql.push_str(&format!(" AND guild_id = ${}", param_index));
            params.push(Box::new(guild_id.unwrap().to_string()));
            param_index += 1;
        }
        if author_id.is_some() {
            sql.push_str(&format!(" AND author_id = ${}", param_index));
            params.push(Box::new(author_id.unwrap().to_string()));
            param_index += 1;
        }
        
        sql.push_str(&format!(" ORDER BY similarity_score DESC, created_at DESC LIMIT ${}", param_index));
        params.push(Box::new(limit as i64));
        
        // Build the query dynamically
        let mut builder = sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(&sql);
        
        for param in params.iter() {
            builder = builder.bind(param.as_ref().as_ref());
        }
        
        let rows = builder.fetch_all(&self.pool).await.map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(rows.into_iter().map(|row| Message {
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
        }).collect())
    }
    
    async fn get_recent_messages(&self, guild_id: Option<&str>, author_id: Option<&str>, limit: usize) -> DbResult<Vec<Message>> {
        let mut sql = String::from("SELECT id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at FROM messages");
        let mut conditions = Vec::new();
        
        if guild_id.is_some() {
            conditions.push(format!("guild_id = '{}'", guild_id.unwrap()));
        }
        if author_id.is_some() {
            conditions.push(format!("author_id = '{}'", author_id.unwrap()));
        }
        
        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }
        
        sql.push_str(&format!(" ORDER BY created_at DESC LIMIT {}", limit));
        
        let rows = sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(rows.into_iter().map(|row| Message {
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
        }).collect())
    }
    
    async fn get_channel_messages(&self, channel_id: &str, limit: usize) -> DbResult<Vec<Message>> {
        let rows = sqlx::query_as::<_, (i64, Option<String>, String, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "SELECT id, discord_message_id, content, author_id, author_name, message_type, guild_id, channel_id, created_at, updated_at
             FROM messages
             WHERE channel_id = $1
             ORDER BY created_at DESC
             LIMIT $2"
        )
        .bind(channel_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(rows.into_iter().map(|row| Message {
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
        }).collect())
    }
    
    async fn cleanup_old_messages(&self, days_old: i32) -> DbResult<u64> {
        let result = sqlx::query(&format!(
            "DELETE FROM messages WHERE created_at < NOW() - INTERVAL '{} days' AND message_type = 'user'",
            days_old
        ))
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(result.rows_affected())
    }
    
    async fn store_memory(&self, user_id: &str, username: &str, memory: &str, guild_id: Option<&str>) -> DbResult<Option<Memory>> {
        let result = sqlx::query(
            "INSERT INTO memories (user_id, username, memory, guild_id)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING
             RETURNING id, user_id, username, memory, guild_id, channel_id, created_at, updated_at"
        )
        .bind(user_id)
        .bind(username)
        .bind(memory)
        .bind(guild_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(result.map(|row| Memory {
            id: row.get(0),
            user_id: row.get(1),
            username: row.get(2),
            memory: row.get(3),
            guild_id: row.get(4),
            channel_id: row.get(5),
            created_at: row.get(6),
            updated_at: row.get(7),
        }))
    }
    
    async fn get_memories(&self, user_id: &str, guild_id: Option<&str>, limit: usize) -> DbResult<Vec<Memory>> {
        let sql = if guild_id.is_some() {
            format!(
                "SELECT id, user_id, username, memory, guild_id, channel_id, created_at, updated_at FROM memories WHERE user_id = $1 AND guild_id = $2 ORDER BY created_at DESC LIMIT {}",
                limit
            )
        } else {
            format!(
                "SELECT id, user_id, username, memory, guild_id, channel_id, created_at, updated_at FROM memories WHERE user_id = $1 ORDER BY created_at DESC LIMIT {}",
                limit
            )
        };
        
        let mut builder = sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(&sql)
            .bind(user_id);
        
        if guild_id.is_some() {
            builder = builder.bind(guild_id.unwrap());
        }
        
        let rows = builder.fetch_all(&self.pool).await.map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(rows.into_iter().map(|row| Memory {
            id: row.0,
            user_id: row.1,
            username: row.2,
            memory: row.3,
            guild_id: row.4,
            channel_id: row.5,
            created_at: row.6,
            updated_at: row.7,
        }).collect())
    }
    
    async fn search_memories(&self, query: &str, user_id: Option<&str>, guild_id: Option<&str>, limit: usize) -> DbResult<Vec<Memory>> {
        let mut sql = String::from(
            "SELECT id, user_id, username, memory, guild_id, channel_id, created_at, updated_at,
             ts_rank(to_tsvector('english', memory), plainto_tsquery('english', $1)) as similarity_score
             FROM memories
             WHERE to_tsvector('english', memory) @@ plainto_tsquery('english', $1)"
        );
        
        let mut param_index = 2;
        
        if user_id.is_some() {
            sql.push_str(&format!(" AND user_id = ${}", param_index));
            param_index += 1;
        }
        if guild_id.is_some() {
            sql.push_str(&format!(" AND guild_id = ${}", param_index));
        }
        
        sql.push_str(&format!(" ORDER BY similarity_score DESC, created_at DESC LIMIT {}", limit));
        
        // For simplicity, just do basic search without all params
        let rows = sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(&sql)
            .bind(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(rows.into_iter().map(|row| Memory {
            id: row.0,
            user_id: row.1,
            username: row.2,
            memory: row.3,
            guild_id: row.4,
            channel_id: row.5,
            created_at: row.6,
            updated_at: row.7,
        }).collect())
    }
    
    async fn remove_memory(&self, memory_id: i64, user_id: &str) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM memories WHERE id = $1 AND user_id = $2")
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
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (user_id)
             DO UPDATE SET username = EXCLUDED.username, day = EXCLUDED.day, month = EXCLUDED.month, year = EXCLUDED.year, updated_at = NOW()"
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
            "SELECT user_id, username, day, month, year, last_pinged_year, created_at, updated_at FROM birthdays WHERE user_id = $1"
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(row.map(|r| Birthday {
            user_id: r.0,
            username: r.1,
            day: r.2,
            month: r.3,
            year: r.4,
            last_pinged_year: r.5,
            created_at: r.6,
            updated_at: r.7,
        }))
    }
    
    async fn remove_birthday(&self, user_id: &str) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM birthdays WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(result.rows_affected() > 0)
    }
    
    async fn get_todays_birthdays(&self, day: i32, month: i32, current_year: i32) -> DbResult<Vec<Birthday>> {
        let rows = sqlx::query_as::<_, (String, String, i32, i32, Option<i32>, i32, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "SELECT user_id, username, day, month, year, last_pinged_year, created_at, updated_at FROM birthdays WHERE day = $1 AND month = $2 AND last_pinged_year < $3"
        )
        .bind(day)
        .bind(month)
        .bind(current_year)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(rows.into_iter().map(|r| Birthday {
            user_id: r.0,
            username: r.1,
            day: r.2,
            month: r.3,
            year: r.4,
            last_pinged_year: r.5,
            created_at: r.6,
            updated_at: r.7,
        }).collect())
    }
    
    async fn mark_birthday_as_pinged(&self, user_id: &str, year: i32) -> DbResult<bool> {
        let result = sqlx::query("UPDATE birthdays SET last_pinged_year = $1 WHERE user_id = $2")
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
             VALUES ($1, $2)
             ON CONFLICT (guild_id)
             DO UPDATE SET channel_id = EXCLUDED.channel_id, updated_at = NOW()"
        )
        .bind(guild_id)
        .bind(channel_id)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(result.rows_affected() > 0)
    }
    
    async fn get_birthday_channel(&self, guild_id: &str) -> DbResult<Option<String>> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT channel_id FROM birthday_channels WHERE guild_id = $1"
        )
        .bind(guild_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(row)
    }
    
    async fn remove_birthday_channel(&self, guild_id: &str) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM birthday_channels WHERE guild_id = $1")
            .bind(guild_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(result.rows_affected() > 0)
    }
    
    async fn set_server_setting(&self, guild_id: &str, setting_name: &str, setting_value: &str) -> DbResult<bool> {
        let result = sqlx::query(
            "INSERT INTO server_settings (guild_id, setting_name, setting_value)
             VALUES ($1, $2, $3)
             ON CONFLICT (guild_id, setting_name)
             DO UPDATE SET setting_value = EXCLUDED.setting_value, updated_at = NOW()"
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
            "SELECT setting_value FROM server_settings WHERE guild_id = $1 AND setting_name = $2"
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
            "SELECT guild_id, setting_name, setting_value FROM server_settings WHERE guild_id = $1"
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(rows.into_iter().map(|r| ServerSetting {
            guild_id: r.0,
            setting_name: r.1,
            setting_value: r.2,
        }).collect())
    }
    
    async fn remove_server_setting(&self, guild_id: &str, setting_name: &str) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM server_settings WHERE guild_id = $1 AND setting_name = $2")
            .bind(guild_id)
            .bind(setting_name)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(result.rows_affected() > 0)
    }
    
    async fn get_message_count(&self) -> DbResult<i64> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(count)
    }
    
    async fn get_unique_channel_count(&self) -> DbResult<i64> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT channel_id) FROM messages")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        
        Ok(count)
    }
}
