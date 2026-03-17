//! veebot - Main entry point
//! A Discord bot that can search online, provide keyword-based help, and more

use serenity::client::bridge::gateway::GatewayIntents;
use serenity::client::Context;
use serenity::framework::StandardFramework;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use veebot::bot::{BotState, handle_interaction, handle_message, handle_ready};
use veebot::config::Config;
use veebot::context::ContextManager;
use veebot::database::DatabaseManager;
use veebot::search::SearchService;

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
    
    // Initialize AI service and check semantic mode
    let is_semantic_mode = Arc::new(RwLock::new(false));
    let ai_service = if let (Some(api_key), Some(model)) = (
        config.openrouter_api_key.as_ref(),
        config.ai_model.as_ref(),
    ) {
        // Initialize database first if enabled
        if config.enable_database {
            if let Err(e) = database.initialize().await {
                tracing::warn!("Database schema init failed: {}", e);
            }
            
            if config.enable_semantic_search {
                if let Ok(connected) = database.test_connection().await {
                    if connected {
                        if let Ok(initialized) = context_manager.initialize().await {
                            *is_semantic_mode.write().await = initialized;
                            if initialized {
                                tracing::info!("Semantic Context Mode ENABLED");
                            }
                        }
                    }
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
    
    // Create Discord client
    let mut client = serenity::Client::builder(
        &config.discord_token,
        GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILD_MESSAGE_REACTIONS
            | GatewayIntents::GUILD_MEMBERS
            | GatewayIntents::GUILD_VOICE_STATES
            | GatewayIntents::GUILD_PRESENCES,
    )
    .framework(StandardFramework::new())
    .type_cache_size(10_000)
    .setup(move |ctx| {
        let state = state.clone();
        ctx.data.insert::<BotState>(state);
        
        // Set up ready handler
        let ctx_clone = ctx.clone();
        Box::pin(async move {
            handle_ready(ctx_clone, serenity::model::gateway::Ready {
                version: 10,
                user: serenity::model::user::CurrentUser::from(
                    serenity::model::id::UserId::new(0),
                    "veebot".to_string(),
                    "",
                    false,
                    serenity::model::user::UserFlags::empty(),
                ),
                guilds: vec![],
                session_id: "".to_string(),
                shard: Some((0, 1)),
                application_id: None,
            }).await;
        })
    })
    .await?;
    
    // Set up event handlers
    let ctx = client.clone();
    client.on_interaction_create(move |ctx, interaction| {
        let ctx = ctx.clone();
        Box::pin(handle_interaction(ctx, interaction))
    });
    
    let ctx = client.clone();
    client.on_message_create(move |ctx, msg| {
        let ctx = ctx.clone();
        Box::pin(handle_message(ctx, msg))
    });
    
    // Start the bot
    tracing::info!("Starting Discord client...");
    client.start().await?;
    
    Ok(())
}
