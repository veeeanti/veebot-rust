//! AI Service module for veebot
//! Handles OpenRouter API integration for AI responses

use crate::context::{ContextManager, ContextMessage};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AiError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),
    #[error("API error: {0}")]
    ApiError(String),
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

pub type AiResult<T> = Result<T, AiError>;

/// Message role for OpenRouter API
#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// OpenRouter message format
#[derive(Debug, Serialize)]
pub struct OpenRouterMessage {
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<ContentPart>>,
}

/// Content part for multimodal messages
#[derive(Debug, Serialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub part_type: String,
    pub text: Option<String>,
    #[serde(rename = "image_url")]
    pub image_url: Option<ImageUrl>,
}

/// Image URL for vision
#[derive(Debug, Serialize)]
pub struct ImageUrl {
    pub url: String,
}

/// Request body for OpenRouter API
#[derive(Debug, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<OpenRouterMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

/// Response from OpenRouter API
#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

/// Choice in the response
#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: ResponseMessage,
}

/// Response message
#[derive(Debug, Deserialize)]
pub struct ResponseMessage {
    pub role: Option<String>,
    pub content: Option<String>,
}

/// Usage statistics
#[derive(Debug, Deserialize)]
pub struct Usage {
    #[serde(rename = "prompt_tokens")]
    pub prompt_tokens: Option<u32>,
    #[serde(rename = "completion_tokens")]
    pub completion_tokens: Option<u32>,
    #[serde(rename = "total_tokens")]
    pub total_tokens: Option<u32>,
}

/// AI Service for generating responses
pub struct AiService {
    client: Client,
    api_key: String,
    model: String,
    context_manager: Arc<ContextManager>,
    prompt: String,
    debug: bool,
    enable_mentions: bool,
}

impl AiService {
    /// Create a new AI service
    pub fn new(
        api_key: String,
        model: String,
        context_manager: Arc<ContextManager>,
        prompt: String,
        debug: bool,
        enable_mentions: bool,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
            context_manager,
            prompt,
            debug,
            enable_mentions,
        }
    }
    
    /// Generate a response using the AI
    pub async fn generate_response(
        &self,
        user_input: &str,
        channel_id: &str,
        guild_id: Option<&str>,
        discord_message_id: &str,
        author_id: &str,
        author_name: &str,
        image_urls: Option<Vec<String>>,
    ) -> AiResult<String> {
        // Build context
        let mut context_text = String::new();
        
        // Get relevant context from the semantic context manager
        let relevant_context = self.context_manager
            .get_relevant_context(user_input, guild_id, Some(author_id))
            .await
            .map_err(|e| AiError::ApiError(e.to_string()))?;
        
        for msg in relevant_context.iter().take(10) {
            let speaker = if msg.msg_type == "assistant" { "AM" } else { &msg.author };
            let similarity = msg.similarity
                .map(|s| format!(" (relevance: {:.1}%)", s * 100.0))
                .unwrap_or_default();
            context_text.push_str(&format!("{}: {}{}\n", speaker, msg.content, similarity));
        }
        
        if self.debug && !context_text.is_empty() {
            tracing::debug!("Used semantic context: {} relevant messages", relevant_context.len());
        }
        
        // Build prompt
        let mut prompt_text = self.prompt.clone();
        
        if !context_text.is_empty() {
            prompt_text.push_str("\n\nContext from conversation:\n");
            prompt_text.push_str(&context_text);
        }
        
        prompt_text.push_str(&format!("\nHuman: {}\nAM:", user_input));
        
        // Build messages
        let mut messages = vec![
            OpenRouterMessage {
                role: Role::System,
                content: Some(self.prompt.clone()),
                parts: None,
            }
        ];
        
        // Check for images
        let has_images = image_urls.as_ref().map(|urls| !urls.is_empty()).unwrap_or(false);
        
        if has_images {
            let mut user_content = Vec::new();
            
            // Add text part
            user_content.push(ContentPart {
                part_type: "text".to_string(),
                text: Some(format!("{}\nKeep your response under 3 sentences.", prompt_text)),
                image_url: None,
            });
            
            // Add image parts
            for url in image_urls.unwrap() {
                user_content.push(ContentPart {
                    part_type: "image_url".to_string(),
                    text: None,
                    image_url: Some(ImageUrl { url }),
                });
            }
            
            messages.push(OpenRouterMessage {
                role: Role::User,
                content: None,
                parts: Some(user_content),
            });
        } else {
            messages.push(OpenRouterMessage {
                role: Role::User,
                content: Some(format!("{}\nKeep your response under 3 sentences.", prompt_text)),
                parts: None,
            });
        }
        
        // Make API request
        let request = ChatCompletionRequest {
            model: self.model.clone(),
            messages,
            temperature: Some(0.5),
            max_tokens: if has_images { Some(300) } else { Some(120) },
        };
        
        if self.debug {
            tracing::debug!("Sending request to OpenRouter");
        }
        
        let response = self.client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;
        
        if self.debug {
            tracing::debug!("Response status: {}", response.status());
        }
        
        let data: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| AiError::ApiError(e.to_string()))?;
        
        let reply = data.choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_else(|| "Your weak words echo in the void.".to_string());
        
        // Clean up the response
        let reply = self.cleanup_response(&reply);
        
        // Store the conversation in context
        if let Err(e) = self.context_manager.store_user_message(
            discord_message_id,
            user_input,
            author_id,
            author_name,
            channel_id,
            guild_id,
        ).await {
            tracing::warn!("Failed to store user message: {}", e);
        }
        
        let assistant_message_id = format!("assistant_{}", discord_message_id);
        if let Err(e) = self.context_manager.store_assistant_message(
            &assistant_message_id,
            &reply,
            channel_id,
            guild_id,
        ).await {
            tracing::warn!("Failed to store assistant message: {}", e);
        }
        
        // Check for important keywords to store as memory
        let important_keywords = ["remember", "remember that", "don't forget", "important", "crucial", "key", "vital"];
        let lower_input = user_input.to_lowercase();
        
        for keyword in important_keywords {
            if lower_input.contains(keyword) {
                let memory_content = lower_input.replace(keyword, "").trim().to_string();
                if memory_content.len() > 5 {
                    if let Err(e) = self.context_manager.store_memory(
                        author_id,
                        author_name,
                        &memory_content,
                        guild_id,
                    ).await {
                        tracing::warn!("Failed to store memory: {}", e);
                    }
                }
                break;
            }
        }
        
        Ok(reply)
    }
    
    /// Clean up the AI response
    fn cleanup_response(&self, response: &str) -> String {
        let mut reply = response.to_string();
        
        // Remove "AM:" prefix if present
        if let Some(idx) = reply.find("AM:") {
            reply = reply[idx + 3..].trim().to_string();
        }
        
        // Remove "Human:" and anything after it
        if let Some(idx) = reply.find("Human:") {
            reply = reply[..idx].trim().to_string();
        }
        
        // Replace newlines with spaces
        reply = reply.replace('\n', " ").trim().to_string();
        
        // Fallback for empty/short responses
        if reply.len() < 3 {
            reply = "Your weak words echo in the void.".to_string();
        }
        
        reply
    }
}
