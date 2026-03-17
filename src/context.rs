//! Context Manager module for veebot
//! Handles semantic memory and context retrieval for AI responses

use crate::database::{DatabaseManager, DbResult, MessageType, StoreMessageData};
use crate::embeddings;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock as TokioRwLock;

/// Context message structure for AI responses
#[derive(Debug, Clone)]
pub struct ContextMessage {
    pub content: String,
    pub author: String,
    pub author_id: Option<String>,
    pub msg_type: String,
    pub similarity: Option<f64>,
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    pub is_recent: bool,
}

/// Statistics about the database
#[derive(Debug, Clone)]
pub struct Statistics {
    pub total_messages: i64,
    pub user_messages: i64,
    pub assistant_messages: i64,
    pub unique_channels: i64,
}

/// Context Manager for semantic memory
pub struct ContextManager {
    database: Arc<DatabaseManager>,
    is_initialized: Arc<TokioRwLock<bool>>,
    message_cache: Arc<RwLock<HashMap<String, Vec<ContextMessage>>>>,
    max_context_messages: usize,
    similarity_threshold: f64,
    enable_semantic_search: bool,
}

impl ContextManager {
    /// Create a new context manager
    pub fn new(
        database: Arc<DatabaseManager>,
        max_context_messages: usize,
        similarity_threshold: f64,
        enable_semantic_search: bool,
    ) -> Self {
        Self {
            database,
            is_initialized: Arc::new(TokioRwLock::new(false)),
            message_cache: Arc::new(RwLock::new(HashMap::new())),
            max_context_messages,
            similarity_threshold,
            enable_semantic_search,
        }
    }
    
    /// Initialize the context manager
    pub async fn initialize(&self) -> DbResult<bool> {
        if !self.enable_semantic_search {
            tracing::info!("Semantic search disabled, skipping context manager init");
            *self.is_initialized.write().await = false;
            return Ok(false);
        }
        
        tracing::info!("Initializing Semantic Context Manager...");
        
        // Perform cleanup of old messages
        if let Err(e) = self.perform_cleanup().await {
            tracing::warn!("Cleanup failed: {}", e);
        }
        
        *self.is_initialized.write().await = true;
        tracing::info!("Semantic Context Manager initialized successfully");
        
        Ok(true)
    }
    
    /// Check if the context manager is ready
    pub async fn is_ready(&self) -> bool {
        *self.is_initialized.read().await
    }
    
    /// Store a user message
    pub async fn store_user_message(
        &self,
        discord_message_id: &str,
        content: &str,
        author_id: &str,
        author_name: &str,
        channel_id: &str,
        guild_id: Option<&str>,
    ) -> DbResult<()> {
        let data = StoreMessageData {
            discord_message_id: discord_message_id.to_string(),
            content: content.to_string(),
            author_id: author_id.to_string(),
            author_name: author_name.to_string(),
            channel_id: channel_id.to_string(),
            guild_id: guild_id.map(|s| s.to_string()),
            message_type: MessageType::User,
        };
        
        self.database.store_message(data).await?;
        
        // Cache for quick access
        let cache_key = discord_message_id.to_string();
        self.message_cache.write().insert(cache_key, vec![ContextMessage {
            content: content.to_string(),
            author: author_name.to_string(),
            author_id: Some(author_id.to_string()),
            msg_type: "user".to_string(),
            similarity: None,
            timestamp: None,
            is_recent: false,
        }]);
        
        Ok(())
    }
    
    /// Store an assistant message
    pub async fn store_assistant_message(
        &self,
        discord_message_id: &str,
        content: &str,
        channel_id: &str,
        guild_id: Option<&str>,
    ) -> DbResult<()> {
        let data = StoreMessageData {
            discord_message_id: discord_message_id.to_string(),
            content: content.to_string(),
            author_id: "assistant".to_string(),
            author_name: "AM".to_string(),
            channel_id: channel_id.to_string(),
            guild_id: guild_id.map(|s| s.to_string()),
            message_type: MessageType::Assistant,
        };
        
        self.database.store_message(data).await?;
        
        // Cache for quick access
        let cache_key = discord_message_id.to_string();
        self.message_cache.write().insert(cache_key, vec![ContextMessage {
            content: content.to_string(),
            author: "AM".to_string(),
            author_id: Some("assistant".to_string()),
            msg_type: "assistant".to_string(),
            similarity: None,
            timestamp: None,
            is_recent: false,
        }]);
        
        Ok(())
    }
    
    /// Store an explicit memory
    pub async fn store_memory(
        &self,
        user_id: &str,
        username: &str,
        memory: &str,
        guild_id: Option<&str>,
    ) -> DbResult<()> {
        self.database.store_memory(user_id, username, memory, guild_id).await?;
        Ok(())
    }
    
    /// Get relevant context for a message
    pub async fn get_relevant_context(
        &self,
        user_input: &str,
        guild_id: Option<&str>,
        user_id: Option<&str>,
    ) -> DbResult<Vec<ContextMessage>> {
        if !self.is_ready().await {
            return Ok(vec![]);
        }
        
        let mut context = Vec::new();
        
        // Check cache first
        let cache_key = format!(
            "{}_{}_{}",
            guild_id.unwrap_or("all_guilds"),
            user_id.unwrap_or("all"),
            user_input
        );
        
        {
            let cache = self.message_cache.read();
            if let Some(cached) = cache.get(&cache_key) {
                return Ok(cached.clone());
            }
        }
        
        if self.enable_semantic_search {
            // Search for similar messages
            let similar_messages = self.database
                .find_similar_messages(
                    user_input,
                    guild_id,
                    user_id,
                    self.max_context_messages / 2,
                )
                .await?;
            
            if !similar_messages.is_empty() {
                context.extend(similar_messages.iter().map(|msg| {
                    let sim = embeddings::calculate_text_similarity(&msg.content, user_input);
                    ContextMessage {
                        content: msg.content.clone(),
                        author: msg.author_name.clone(),
                        author_id: Some(msg.author_id.clone()),
                        msg_type: match msg.message_type {
                            MessageType::Assistant => "assistant".to_string(),
                            _ => "user".to_string(),
                        },
                        similarity: Some(sim),
                        timestamp: Some(msg.created_at),
                        is_recent: false,
                    }
                }));
                
                tracing::debug!("Found {} textually relevant messages", context.len());
            }
            
            // Search for relevant memories
            if let Some(uid) = user_id {
                let relevant_memories = self.database
                    .search_memories(user_input, Some(uid), guild_id, self.max_context_messages / 4)
                    .await?;
                
                if !relevant_memories.is_empty() {
                    context.extend(relevant_memories.iter().map(|mem| {
                        let sim = embeddings::calculate_text_similarity(&mem.memory, user_input);
                        ContextMessage {
                            content: format!("[Memory] {}", mem.memory),
                            author: mem.username.clone(),
                            author_id: Some(mem.user_id.clone()),
                            msg_type: "memory".to_string(),
                            similarity: Some(sim),
                            timestamp: Some(mem.created_at),
                            is_recent: false,
                        }
                    }));
                    
                    tracing::debug!("Found {} relevant memories", relevant_memories.len());
                }
            }
        }
        
        // Add recent messages if context is limited
        if context.len() < self.max_context_messages / 2 {
            let recent_messages = self.database
                .get_recent_messages(guild_id, user_id, self.max_context_messages - context.len())
                .await?;
            
            for msg in recent_messages {
                // Avoid duplicates
                if !context.iter().any(|c| c.content == msg.content) {
                    context.push(ContextMessage {
                        content: msg.content.clone(),
                        author: msg.author_name.clone(),
                        author_id: Some(msg.author_id.clone()),
                        msg_type: match msg.message_type {
                            MessageType::Assistant => "assistant".to_string(),
                            _ => "user".to_string(),
                        },
                        similarity: None,
                        timestamp: Some(msg.created_at),
                        is_recent: true,
                    });
                }
            }
        }
        
        // Sort by relevance (similarity first, then recency)
        context.sort_by(|a, b| {
            match (a.similarity, b.similarity) {
                (Some(sim_a), Some(sim_b)) => sim_b.partial_cmp(&sim_a).unwrap(),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => {
                    match (a.timestamp, b.timestamp) {
                        (Some(t_a), Some(t_b)) => t_b.cmp(&t_a),
                        _ => std::cmp::Ordering::Equal,
                    }
                }
            }
        });
        
        // Cache result
        let final_context: Vec<ContextMessage> = context.iter().take(self.max_context_messages).cloned().collect();
        self.message_cache.write().insert(cache_key, final_context.clone());
        
        Ok(final_context)
    }
    
    /// Get channel context for multi-person chats
    pub async fn get_channel_context(&self, channel_id: &str, limit: usize) -> DbResult<Vec<ContextMessage>> {
        let messages = self.database.get_channel_messages(channel_id, limit).await?;
        
        let result: Vec<ContextMessage> = messages
            .into_iter()
            .rev()
            .map(|msg| ContextMessage {
                content: msg.content,
                author: msg.author_name,
                author_id: Some(msg.author_id),
                msg_type: match msg.message_type {
                    MessageType::Assistant => "assistant".to_string(),
                    _ => "user".to_string(),
                },
                similarity: None,
                timestamp: Some(msg.created_at),
                is_recent: false,
            })
            .collect();
        
        Ok(result)
    }
    
    /// Perform cleanup of old messages
    async fn perform_cleanup(&self) -> DbResult<u64> {
        let cleaned = self.database.cleanup_old_messages(30).await?;
        if cleaned > 0 {
            tracing::info!("Cleaned up {} old messages", cleaned);
        }
        Ok(cleaned)
    }
    
    /// Get database statistics
    pub async fn get_statistics(&self) -> DbResult<Statistics> {
        let total = self.database.get_message_count().await?;
        let channels = self.database.get_unique_channel_count().await?;
        
        Ok(Statistics {
            total_messages: total,
            user_messages: 0, // Not tracked separately in this simplified version
            assistant_messages: 0,
            unique_channels: channels,
        })
    }
}
