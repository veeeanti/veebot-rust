//! veebot - Main entry point
//! A Discord bot that can search online, provide keyword-based help, and more

use serenity::all::{Context, GatewayIntents, Interaction, Message, Ready};
use serenity::prelude::EventHandler;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use veebot::bot::{BotState, handle_interaction, handle_message, handle_ready};
use veebot::config::DatabaseType;
use veebot::config::Config;
use veebot::context::ContextManager;
use veebot::database::DatabaseManager;
use veebot::search::SearchService;

struct Handler {
    state: Arc<BotState>,
}

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        handle_ready(ctx, ready, self.state.clone()).await;
    }

    async fn message(&self, ctx: Context, msg: Message) {
        handle_message(ctx, msg, self.state.clone()).await;
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        handle_interaction(ctx, interaction, self.state.clone()).await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(EnvFilter::new("info"))
        .with(fmt::layer())
        .init();
    
    tracing::info!("Starting veebot...");
    
    // Load configuration
    let config = Arc::new(Config::from_env().map_err(|e| format!("Config error: {}", e))?);
    
    if config.discord_token.is_empty() {
        return Err("DISCORD_TOKEN not set".into());
    }
    
    tracing::info!("Configuration loaded");
    
    // Initialize database
    let database = Arc::new(
        DatabaseManager::new(&config)
            .await
            .map_err(|e| format!("Database init error: {}", e))?
    );
    
    // Initialize context manager
    let context_manager = Arc::new(ContextManager::new(
        database.clone(),
        config.max_context_messages,
        config.context_similarity_threshold,
        config.enable_semantic_search,
    ));
    
    // Initialize search service
    let search_service = Arc::new(SearchService::new(config.search_engine.clone()));
    
    // Initialize AI service and semantic mode
    let is_semantic_mode = Arc::new(RwLock::new(false));
    let ai_service = if let (Some(api_key), Some(model)) = (
        config.openrouter_api_key.as_ref(),
        config.ai_model.as_ref(),
    ) {
        if config.enable_database {
            if let Err(e) = database.initialize().await {
                tracing::warn!("Database schema init failed: {}", e);
            } else if config.enable_semantic_search {
                match database.test_connection().await {
                    Ok(true) => match context_manager.initialize().await {
                        Ok(initialized) => {
                            *is_semantic_mode.write().await = initialized;
                            if initialized {
                                tracing::info!("Semantic Context Mode ENABLED");
                            }
                        }
                        Err(e) => tracing::warn!("Context manager init failed: {}", e),
                    },
                    Ok(false) => tracing::warn!("Database connection test failed"),
                    Err(e) => tracing::warn!("Database connection test failed: {}", e),
                }
            }
        }
        
        Some(veebot::ai::AiService::new(
            api_key.clone(),
            model.clone(),
            context_manager.clone(),
            config.prompt.clone(),
            config.debug,
            config.enable_mentions,
        ))
    } else {
        tracing::warn!("AI service not configured - AI commands will not work");
        None
    };
    
    // Create bot state
    let state = Arc::new(BotState {
        config: config.clone(),
        database: database.clone(),
        context_manager: context_manager.clone(),
        ai_service: Arc::new(ai_service),
        search_service,
        is_semantic_mode,
        start_time: Instant::now(),
        last_response_time: Arc::new(RwLock::new(0)),
    });
    
    if !config.enable_database && config.database_type == DatabaseType::Postgres {
        tracing::warn!("Postgres is configured but database support is disabled");
    }

    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::GUILD_VOICE_STATES
        | GatewayIntents::GUILD_PRESENCES;

    // Create Discord client
    let mut client = serenity::Client::builder(
        &config.discord_token,
        intents,
    )
    .event_handler(Handler { state })
    .await?;
    
    // Start the bot
    tracing::info!("Starting Discord client...");
    client.start().await?;
    
    Ok(())
}
