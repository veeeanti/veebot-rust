//! Bot module for veebot
//! Handles Discord interactions and commands

use crate::ai::AiService;
use crate::config::Config;
use crate::context::ContextManager;
use crate::database::DatabaseManager;
use crate::search::SearchService;
use serenity::builder::{CreateButton, CreateEmbed, CreateInteractionResponseFollowup};
use serenity::model::application::{ButtonStyle, Command, CommandOptionType, ComponentType};
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::{ChannelId, GuildId, UserId};
use serenity::model::permissions::Permissions;
use serenity::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Bot state
pub struct BotState {
    pub config: Arc<Config>,
    pub database: Arc<DatabaseManager>,
    pub context_manager: Arc<ContextManager>,
    pub ai_service: Arc<Option<AiService>>,
    pub search_service: Arc<SearchService>,
    pub is_semantic_mode: Arc<RwLock<bool>>,
    pub start_time: Instant,
    pub last_response_time: Arc<RwLock<u64>>,
}

/// Initialize the bot
pub async fn initialize_bot(
    config: Arc<Config>,
    database: Arc<DatabaseManager>,
    context_manager: Arc<ContextManager>,
) -> Result<Arc<Option<AiService>>, String> {
    let mut is_semantic_mode = false;
    
    if !config.enable_database {
        tracing::info!("Database DISABLED - Running in Simple Mode");
        return Ok(Arc::new(None));
    }
    
    if config.enable_semantic_search {
        let db_connected = database.test_connection().await
            .map_err(|e| format!("Database connection failed: {}", e))?;
        
        if !db_connected {
            tracing::warn!("Database connection failed, falling back to simple mode");
            return Ok(Arc::new(None));
        }
        
        database.initialize().await
            .map_err(|e| format!("Database schema init failed: {}", e))?;
        
        // Initialize context manager
        let initialized = context_manager.initialize().await
            .map_err(|e| format!("Context manager init failed: {}", e))?;
        
        if initialized {
            is_semantic_mode = true;
            tracing::info!("Semantic Context Mode ENABLED");
        } else {
            tracing::warn!("Context manager init failed, falling back to simple mode");
        }
    }
    
    // Create AI service if configured
    let ai_service = if let (Some(api_key), Some(model)) = (
        config.openrouter_api_key.as_ref(),
        config.ai_model.as_ref(),
    ) {
        let service = AiService::new(
            api_key.clone(),
            model.clone(),
            context_manager,
            config.prompt.clone(),
            config.debug,
            config.enable_mentions,
        );
        tracing::info!("AI Service initialized with model: {}", model);
        Some(service)
    } else {
        None
    };
    
    Ok(Arc::new(ai_service))
}

/// Set up slash commands
pub async fn setup_commands(ctx: &Context) -> Result<(), serenity::Error> {
    let commands = vec![
        ("search", "Search UnionCrax for games", vec![
            ("query", "The game to search for", CommandOptionType::String, true),
        ]),
        ("info", "Get information about the bot", vec![]),
        ("birthday", "Manage your birthday", vec![
            ("set", "Set your birthday", vec![
                ("month", "Month (1-12)", CommandOptionType::Integer, true),
                ("day", "Day (1-31)", CommandOptionType::Integer, true),
                ("year", "Year (optional)", CommandOptionType::Integer, false),
                ("user", "User to set birthday for", CommandOptionType::User, false),
            ]),
            ("remove", "Remove your birthday", vec![
                ("user", "User to remove birthday for", CommandOptionType::User, false),
            ]),
            ("get", "Get your birthday", vec![
                ("user", "User to get birthday for", CommandOptionType::User, false),
            ]),
            ("test", "Test birthday announcements", vec![
                ("day", "Day to test", CommandOptionType::Integer, false),
                ("month", "Month to test", CommandOptionType::Integer, false),
            ]),
            ("send", "Send today's birthday pings", vec![]),
            ("channel", "Manage birthday channel", vec![
                ("set", "Set birthday channel", vec![
                    ("channel", "Channel to set", CommandOptionType::Channel, true),
                ]),
                ("remove", "Remove birthday channel", vec![]),
                ("get", "Get birthday channel", vec![]),
            ]),
        ]),
        ("settings", "Manage server settings", vec![
            ("view", "View all server settings", vec![]),
            ("ai_channel", "Set AI response channel", vec![
                ("channel", "Channel for AI responses", CommandOptionType::Channel, false),
            ]),
            ("response_chance", "Set response chance", vec![
                ("percentage", "0.0 to 1.0", CommandOptionType::String, true),
            ]),
        ]),
        ("location", "Get bot location info", vec![]),
        ("ping", "Check bot latency", vec![]),
        ("ask", "Ask the AI a question", vec![
            ("question", "Your question", CommandOptionType::String, true),
        ]),
        ("stats", "Show server statistics", vec![]),
        ("help", "Show available commands", vec![]),
    ];
    
    for (name, description, options) in commands {
        let mut builder = Command::create_global_command(
            ctx,
            serenity::builder::CreateCommand::new(name)
                .description(description)
                .dm_permission(true)
                .default_member_permissions(Permissions::empty()),
        );
        
        // Add options
        for (opt_name, opt_desc, opt_type, required) in options {
            builder = builder.add_option(
                serenity::builder::CreateCommandOption::new(opt_type, opt_name, opt_desc)
                    .required(required),
            );
        }
        
        builder.execute(ctx).await?;
    }
    
    tracing::info!("Slash commands registered");
    Ok(())
}

/// Handle interaction events
pub async fn handle_interaction(ctx: Context, interaction: serenity::model::application::Interaction) {
    use serenity::model::application::InteractionData;
    
    let data = match &interaction.data() {
        Some(d) => d,
        None => return,
    };
    
    let state = match ctx.data.get::<BotState>() {
        Some(s) => s.clone(),
        None => return,
    };
    
    match data {
        InteractionData::ApplicationCommand(cmd) => {
            let response = match cmd.data.name.as_str() {
                "search" => handle_search(&ctx, &cmd, &state).await,
                "info" => handle_info(&ctx, &cmd, &state).await,
                "ping" => handle_ping(&ctx, &cmd).await,
                "ask" => handle_ask(&ctx, &cmd, &state).await,
                "stats" => handle_stats(&ctx, &cmd).await,
                "help" => handle_help(&ctx, &cmd).await,
                "birthday" => handle_birthday(&ctx, &cmd, &state).await,
                "settings" => handle_settings(&ctx, &cmd, &state).await,
                "location" => handle_location(&ctx, &cmd).await,
                _ => cmd.callback().edit_original_interaction_response(&ctx, |r| {
                    r.content("Unknown command")
                }),
            };
            
            if let Err(e) = response {
                tracing::error!("Error handling command: {}", e);
            }
        }
        _ => {}
    }
}

// Command handlers
async fn handle_search(ctx: &Context, cmd: &Command, state: &Arc<BotState>) -> Result<(), serenity::Error> {
    let query = cmd.data.options.first()
        .and_then(|o| o.value.as_str())
        .unwrap_or("");
    
    cmd.callback().defer(ctx).await?;
    
    let result = state.search_service.search_google_for_union_crax(query).await;
    
    match result {
        Ok(Some(game)) => {
            let embed = CreateEmbed::new()
                .title(&game.title)
                .url(&game.url)
                .description(game.description.as_deref().unwrap_or("No description"))
                .field("Source", &game.source, true)
                .field("Views", game.view_count.map(|v| v.to_string()).unwrap_or_else(|| "N/A".to_string()), true)
                .field("Downloads", game.download_count.map(|d| d.to_string()).unwrap_or_else(|| "N/A".to_string()), true)
                .field("Size", game.size.as_deref().unwrap_or("N/A"), true);
            
            let button = CreateButton::new("download")
                .label("Download Game")
                .style(ButtonStyle::Link)
                .url(&game.url);
            
            cmd.callback().edit_original_interaction_response(ctx, |r| {
                r.embeds().push(embed);
                r.components().push(serenity::builder::CreateActionRow::Button(button))
            }).await
        }
        Ok(None) => {
            cmd.callback().edit_original_interaction_response(ctx, |r| {
                r.content(format!("No matching games found for: **{}**", query))
            }).await
        }
        Err(e) => {
            cmd.callback().edit_original_interaction_response(ctx, |r| {
                r.content(format!("Error: {}", e))
            }).await
        }
    }
}

async fn handle_info(ctx: &Context, cmd: &Command, state: &Arc<BotState>) -> Result<(), serenity::Error> {
    let uptime = state.start_time.elapsed();
    let is_semantic = *state.is_semantic_mode.read().await;
    
    let mut embed = CreateEmbed::new()
        .title("🤖 UC-AIv2 Info")
        .field("Model", state.config.ai_model.as_deref().unwrap_or("Not configured"), true)
        .field("Mode", if is_semantic { "🔮 Semantic" } else { "💬 Simple" }, true)
        .field("Uptime", format!("{}h {}m {}s", uptime.as_secs() / 3600, (uptime.as_secs() % 3600) / 60, uptime.as_secs() % 60), true)
        .field("Database", if state.config.enable_database { format!("✅ Enabled ({})", state.config.database_type.to_string()) } else { "❌ Disabled".to_string() }, true)
        .field("Mentions", if state.config.enable_mentions { "✅ Enabled" } else { "❌ Disabled" }, true)
        .field("Semantic", if state.config.enable_semantic_search { "✅ Enabled" } else { "❌ Disabled" }, true);
    
    // Add database stats if semantic mode
    if is_semantic && state.config.enable_database {
        if let Ok(stats) = state.context_manager.get_statistics().await {
            embed = embed
                .field("Total Messages", stats.total_messages.to_string(), true)
                .field("Channels", stats.unique_channels.to_string(), true);
        }
    }
    
    cmd.callback().create_interaction_response(ctx, |r| {
        r.embed(|e| e.0 = embed.0)
    }).await
}

async fn handle_ping(ctx: &Context, cmd: &Command) -> Result<(), serenity::Error> {
    let sent = cmd.callback().create_interaction_response(ctx, |r| {
        r.interaction_response_data(|d| d.content("🏓 Pinging..."))
    }).await?;
    
    let roundtrip = sent.id.get().timestamp_ms() as i64 - cmd.id.get().timestamp_ms() as i64;
    let ws_latency = ctx.cache.latency().avg().unwrap_or_default().as_millis() as i64;
    
    cmd.callback().edit_original_interaction_response(ctx, |r| {
        r.embeds().push(CreateEmbed::new()
            .title("🏓 Pong!")
            .field("Roundtrip Latency", format!("{}ms", roundtrip), true)
            .field("WebSocket Heartbeat", format!("{}ms", ws_latency), true)
            .color(if ws_latency < 100 { 0x00FF00 } else if ws_latency < 250 { 0xFFFF00 } else { 0xFF0000 }))
    }).await
}

async fn handle_ask(ctx: &Context, cmd: &Command, state: &Arc<BotState>) -> Result<(), serenity::Error> {
    let ai_service = match state.ai_service.as_ref() {
        Some(s) => s,
        None => {
            return cmd.callback().create_interaction_response(ctx, |r| {
                r.interaction_response_data(|d| d.content("AI service not configured"))
            }).await;
        }
    };
    
    let question = cmd.data.options.first()
        .and_then(|o| o.value.as_str())
        .unwrap_or("");
    
    cmd.callback().defer(ctx).await?;
    
    let answer = ai_service.generate_response(
        question,
        &cmd.channel_id.to_string(),
        cmd.guild_id.as_ref().map(|g| g.to_string()).as_deref(),
        &format!("slash_{}", cmd.id),
        &cmd.user.id.to_string(),
        &cmd.user.name,
        None,
    ).await;
    
    match answer {
        Ok(reply) => {
            cmd.callback().edit_original_interaction_response(ctx, |r| {
                r.embeds().push(CreateEmbed::new()
                    .title("🤖 AI Response")
                    .field("❓ Question", &question[..question.len().min(1024)], false)
                    .field("💬 Answer", &reply[..reply.len().min(1024)], false)
                    .footer(|f| f.text(format!("Asked by {}", cmd.user.name)))
                    .color(0x5865F2))
            }).await
        }
        Err(e) => {
            cmd.callback().edit_original_interaction_response(ctx, |r| {
                r.content(format!("Error: {}", e))
            }).await
        }
    }
}

async fn handle_stats(ctx: &Context, cmd: &Command) -> Result<(), serenity::Error> {
    let guild = match cmd.guild_id {
        Some(id) => ctx.cache.guild(id),
        None => {
            return cmd.callback().create_interaction_response(ctx, |r| {
                r.interaction_response_data(|d| d.content("This command can only be used in a server"))
            }).await;
        }
    };
    
    let guild = match guild {
        Some(g) => g,
        None => return Ok(()),
    };
    
    let (total, online, bots) = {
        let members = guild.members(ctx, None, None).await.ok();
        let members = members.as_ref().map(|m| m.len()).unwrap_or(0);
        let bots = guild.members(ctx, None, None).await.ok()
            .map(|m| m.iter().filter(|m| m.user.bot).count())
            .unwrap_or(0);
        (members, members - bots, bots)
    };
    
    cmd.callback().create_interaction_response(ctx, |r| {
        r.embed(|e| {
            e.title(format!("📊 {} — Server Stats", guild.name))
                .field("Total Members", total.to_string(), true)
                .field("Humans", online.to_string(), true)
                .field("Bots", bots.to_string(), true)
                .field("Channels", guild.channels(ctx).await.len().to_string(), true)
                .field("Roles", guild.roles.len().to_string(), true)
                .color(0x5865F2)
        })
    }).await
}

async fn handle_help(ctx: &Context, cmd: &Command) -> Result<(), serenity::Error> {
    let embed = CreateEmbed::new()
        .title("📖 Available Commands")
        .description("Here are all the slash commands you can use:")
        .field("🔍 `/search <query>`", "Search UnionCrax for games", false)
        .field("🤖 `/ask <question>`", "Ask the AI a question", false)
        .field("ℹ️ `/info`", "Show bot information", false)
        .field("📊 `/stats`", "Display server statistics", false)
        .field("🏓 `/ping`", "Check bot latency", false)
        .field("📍 `/location`", "Show runtime details", false)
        .field("🎂 `/birthday`", "Manage birthdays", false)
        .field("⚙️ `/settings`", "Manage server settings", false)
        .field("📖 `/help`", "Show this help message", false)
        .color(0x5865F2);
    
    cmd.callback().create_interaction_response(ctx, |r| {
        r.embed(|e| e.0 = embed.0)
    }).await
}

async fn handle_birthday(ctx: &Context, cmd: &Command, state: &Arc<BotState>) -> Result<(), serenity::Error> {
    use crate::database::StoreMessageData;
    
    let subcommand = cmd.data.options.first()
        .and_then(|o| o.name.as_str());
    
    match subcommand {
        Some("set") => {
            let month = cmd.data.options.first()
                .and_then(|o| o.options.first())
                .and_then(|o| o.value.as_i64())
                .unwrap_or(0) as i32;
            let day = cmd.data.options.first()
                .and_then(|o| o.options.get(1))
                .and_then(|o| o.value.as_i64())
                .unwrap_or(0) as i32;
            let year = cmd.data.options.first()
                .and_then(|o| o.options.get(2))
                .and_then(|o| o.value.as_i64())
                .map(|y| y as i32);
            
            if month == 0 && day == 0 && year.unwrap_or(0) == 0 {
                return cmd.callback().create_interaction_response(ctx, |r| {
                    r.interaction_response_data(|d| d.content("Do you even exist??"))
                }).await;
            }
            
            let target = cmd.data.options.first()
                .and_then(|o| o.options.get(3))
                .and_then(|o| o.value.as_user_id())
                .map(|u| (u.to_string(), u.to_string()))
                .unwrap_or_else(|| (cmd.user.id.to_string(), cmd.user.name.clone()));
            
            let success = state.database.set_birthday(
                &target.0,
                &target.1,
                day,
                month,
                year,
            ).await.is_ok();
            
            cmd.callback().create_interaction_response(ctx, |r| {
                r.interaction_response_data(|d| {
                    if success {
                        d.content(format!("✅ Birthday set to **{}/{}{}**", month, day, year.map(|y| format!(", {}", y)).unwrap_or_default()))
                    } else {
                        d.content("❌ Failed to save birthday")
                    }
                })
            }).await
        }
        Some("get") => {
            let user_id = cmd.data.options.first()
                .and_then(|o| o.options.first())
                .and_then(|o| o.value.as_user_id())
                .map(|u| u.to_string())
                .unwrap_or_else(|| cmd.user.id.to_string());
            
            if let Ok(Some(bday)) = state.database.get_birthday(&user_id).await {
                cmd.callback().create_interaction_response(ctx, |r| {
                    r.interaction_response_data(|d| {
                        d.content(format!("🎂 Birthday: **{}/{}{}**", 
                            bday.month, bday.day, bday.year.map(|y| format!("/{}", y)).unwrap_or_default()))
                    })
                }).await
            } else {
                cmd.callback().create_interaction_response(ctx, |r| {
                    r.interaction_response_data(|d| d.content("❌ No birthday found"))
                }).await
            }
        }
        Some("remove") => {
            let user_id = cmd.data.options.first()
                .and_then(|o| o.options.first())
                .and_then(|o| o.value.as_user_id())
                .map(|u| u.to_string())
                .unwrap_or_else(|| cmd.user.id.to_string());
            
            let success = state.database.remove_birthday(&user_id).await.is_ok();
            
            cmd.callback().create_interaction_response(ctx, |r| {
                r.interaction_response_data(|d| {
                    if success {
                        d.content("✅ Birthday removed")
                    } else {
                        d.content("❌ Failed to remove birthday")
                    }
                })
            }).await
        }
        _ => {
            cmd.callback().create_interaction_response(ctx, |r| {
                r.interaction_response_data(|d| d.content("Use /help to see birthday options"))
            }).await
        }
    }
}

async fn handle_settings(ctx: &Context, cmd: &Command, state: &Arc<BotState>) -> Result<(), serenity::Error> {
    let guild_id = match cmd.guild_id {
        Some(g) => g.to_string(),
        None => {
            return cmd.callback().create_interaction_response(ctx, |r| {
                r.interaction_response_data(|d| d.content("This command can only be used in a server"))
            }).await;
        }
    };
    
    let subcommand = cmd.data.options.first()
        .and_then(|o| o.name.as_str());
    
    match subcommand {
        Some("view") => {
            let settings = state.database.get_all_server_settings(&guild_id).await.unwrap_or_default();
            
            let ai_channel = settings.iter()
                .find(|s| s.setting_name == "ai_channel")
                .and_then(|s| s.setting_value.clone())
                .unwrap_or_else(|| "Not set".to_string());
            
            let response_chance = settings.iter()
                .find(|s| s.setting_name == "random_response_chance")
                .and_then(|s| s.setting_value.clone())
                .unwrap_or_else(|| "0.1".to_string());
            
            cmd.callback().create_interaction_response(ctx, |r| {
                r.embed(|e| {
                    e.title("⚙️ Server Settings")
                        .field("AI Channel", ai_channel, true)
                        .field("Response Chance", format!("{}%", (response_chance.parse::<f64>().unwrap_or(0.1) * 100.0) as i32), true)
                        .color(0x5865F2)
                })
            }).await
        }
        Some("ai_channel") => {
            let channel_id = cmd.data.options.first()
                .and_then(|o| o.options.first())
                .and_then(|o| o.value.as_channel_id())
                .map(|c| c.to_string());
            
            if let Some(ch_id) = channel_id {
                state.database.set_server_setting(&guild_id, "ai_channel", &ch_id).await.ok();
                cmd.callback().create_interaction_response(ctx, |r| {
                    r.interaction_response_data(|d| d.content(format!("✅ AI channel set")))
                }).await
            } else {
                state.database.remove_server_setting(&guild_id, "ai_channel").await.ok();
                cmd.callback().create_interaction_response(ctx, |r| {
                    r.interaction_response_data(|d| d.content("✅ AI channel reset"))
                }).await
            }
        }
        _ => Ok(())
    }
}

async fn handle_location(ctx: &Context, cmd: &Command) -> Result<(), serenity::Error> {
    let mem = std::mem::size_of::<()>();
    
    cmd.callback().create_interaction_response(ctx, |r| {
        r.embed(|e| {
            e.title("📍 Bot Location Information")
                .field("Platform", std::env::consts::OS, true)
                .field("Architecture", std::env::consts::ARCH, true)
                .color(0x0099FF)
        })
    }).await
}

// Helper extension for Option
trait OptionExt {
    fn as_channel_id(&self) -> Option<ChannelId>;
    fn as_user_id(&self) -> Option<UserId>;
    fn as_i64(&self) -> Option<i64>;
    fn as_str(&self) -> Option<&str>;
}

impl OptionExt for serenity::model::application::CommandDataOptionValue {
    fn as_channel_id(&self) -> Option<ChannelId> {
        match self {
            serenity::model::application::CommandDataOptionValue::Channel(c) => Some(*c),
            _ => None,
        }
    }
    fn as_user_id(&self) -> Option<UserId> {
        match self {
            serenity::model::application::CommandDataOptionValue::User(u, _) => Some(*u),
            _ => None,
        }
    }
    fn as_i64(&self) -> Option<i64> {
        match self {
            serenity::model::application::CommandDataOptionValue::Integer(i) => Some(*i),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            serenity::model::application::CommandDataOptionValue::String(s) => Some(s),
            _ => None,
        }
    }
}

// Message handler
pub async fn handle_message(ctx: Context, msg: Message) {
    let state = match ctx.data.get::<BotState>() {
        Some(s) => s.clone(),
        None => return,
    };
    
    // Ignore bot messages
    if msg.author.bot && !state.config.friendly_fire {
        return;
    }
    
    // Store message in database
    if state.config.enable_database {
        if let Err(e) = state.database.store_message(crate::database::StoreMessageData {
            discord_message_id: msg.id.to_string(),
            content: msg.content.clone(),
            author_id: msg.author.id.to_string(),
            author_name: msg.author.name.clone(),
            channel_id: msg.channel_id.to_string(),
            guild_id: msg.guild_id.map(|g| g.to_string()),
            message_type: crate::database::MessageType::User,
        }).await {
            tracing::warn!("Failed to store message: {}", e);
        }
    }
    
    // Check if we should respond
    let should_respond = {
        let mentioned = msg.mentions.iter().any(|u| u.id == ctx.cache.current_user_id());
        let correct_channel = state.config.channel_id.as_ref()
            .map(|c| c == &msg.channel_id.to_string())
            .unwrap_or(false);
        
        if mentioned {
            true
        } else if correct_channel {
            let last_time = *state.last_response_time.read().await;
            let current_time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            
            if current_time - last_time > 10000 {
                let chance = state.config.random_response_chance;
                rand_simple() < chance
            } else {
                false
            }
        } else {
            false
        }
    };
    
    if should_respond && state.ai_service.is_some() {
        let ai_service = state.ai_service.as_ref().unwrap();
        
        // Get image attachments
        let image_urls: Option<Vec<String>> = msg.attachments.iter()
            .filter(|a| a.content_type.as_ref().map(|c| c.starts_with("image/")).unwrap_or(false))
            .map(|a| a.url.clone())
            .collect::<Vec<_>>()
            .into();
        
        // Wait a bit before responding
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        let _ = msg.channel_id.broadcast_typing(&ctx);
        
        let reply = ai_service.generate_response(
            &msg.content,
            &msg.channel_id.to_string(),
            msg.guild_id.map(|g| g.to_string()).as_deref(),
            &msg.id.to_string(),
            &msg.author.id.to_string(),
            &msg.author.name,
            image_urls,
        ).await;
        
        if let Ok(response) = reply {
            let _ = msg.reply(&ctx, &response).await;
            
            // Update last response time
            let current_time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            *state.last_response_time.write().await = current_time;
        }
    }
}

fn rand_simple() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    (nanos as f64) / (u32::MAX as f64)
}

// Ready event handler
pub async fn handle_ready(ctx: Context, ready: Ready) {
    tracing::info!("Logged in as {}", ready.user.name);
    
    if let Err(e) = setup_commands(&ctx).await {
        tracing::error!("Failed to setup commands: {}", e);
    }
}
