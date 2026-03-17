//! Bot module for veebot
//! Handles Discord interactions and commands

use crate::ai::AiService;
use crate::config::Config;
use crate::context::ContextManager;
use crate::database::DatabaseManager;
use crate::search::SearchService;
use chrono::Datelike;
use serenity::all::{
    ChannelId, Command, CommandDataOption, CommandDataOptionValue, CommandInteraction,
    CommandOptionType, Context, CreateActionRow, CreateButton, CreateCommand,
    CreateCommandOption, CreateEmbed, CreateInteractionResponse,
    CreateInteractionResponseMessage, EditInteractionResponse, Interaction, Message, Ready,
    UserId,
};
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
    Command::set_global_commands(&ctx.http, build_commands()).await?;
    
    tracing::info!("Slash commands registered");
    Ok(())
}

/// Handle interaction events
pub async fn handle_interaction(ctx: Context, interaction: Interaction, state: Arc<BotState>) {
    if let Interaction::Command(command) = interaction {
        let result = match command.data.name.as_str() {
            "search" => handle_search(&ctx, &command, &state).await,
            "info" => handle_info(&ctx, &command, &state).await,
            "ping" => handle_ping(&ctx, &command).await,
            "ask" => handle_ask(&ctx, &command, &state).await,
            "stats" => handle_stats(&ctx, &command).await,
            "help" => handle_help(&ctx, &command).await,
            "birthday" => handle_birthday(&ctx, &command, &state).await,
            "settings" => handle_settings(&ctx, &command, &state).await,
            "location" => handle_location(&ctx, &command).await,
            _ => respond_text(&ctx, &command, "Unknown command").await,
        };

        if let Err(e) = result {
            tracing::error!("Error handling command {}: {}", command.data.name, e);
        }
    }
}

// Command handlers
async fn handle_search(ctx: &Context, cmd: &CommandInteraction, state: &BotState) -> Result<(), serenity::Error> {
    let query = string_option(&cmd.data.options, "query").unwrap_or_default();

    cmd.defer(ctx).await?;
    
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
            
            let button = CreateButton::new_link(&game.url).label("Download Game");
            let components = vec![CreateActionRow::Buttons(vec![button])];

            cmd.edit_response(
                ctx,
                EditInteractionResponse::new().embed(embed).components(components),
            )
            .await
            .map(|_| ())
        }
        Ok(None) => edit_text(ctx, cmd, format!("No matching games found for: **{}**", query)).await,
        Err(e) => edit_text(ctx, cmd, format!("Error: {}", e)).await,
    }
}

async fn handle_info(ctx: &Context, cmd: &CommandInteraction, state: &BotState) -> Result<(), serenity::Error> {
    let uptime = state.start_time.elapsed();
    let is_semantic = *state.is_semantic_mode.read().await;
    
    let mut embed = CreateEmbed::new()
        .title("🤖 UC-AIv2 Info")
        .field("Model", state.config.ai_model.as_deref().unwrap_or("Not configured"), true)
        .field("Mode", if is_semantic { "🔮 Semantic" } else { "💬 Simple" }, true)
        .field("Uptime", format!("{}h {}m {}s", uptime.as_secs() / 3600, (uptime.as_secs() % 3600) / 60, uptime.as_secs() % 60), true)
        .field("Database", if state.config.enable_database { format!("Enabled ({})", state.config.database_type) } else { "Disabled".to_string() }, true)
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
    
    respond_embed(ctx, cmd, embed).await
}

async fn handle_ping(ctx: &Context, cmd: &CommandInteraction) -> Result<(), serenity::Error> {
    let started = Instant::now();
    respond_text(ctx, cmd, "🏓 Pinging...").await?;

    let roundtrip_ms = started.elapsed().as_millis();
    let embed = CreateEmbed::new()
        .title("🏓 Pong!")
        .field("Roundtrip Latency", format!("{}ms", roundtrip_ms), true)
        .field("Gateway Heartbeat", "Cache unavailable".to_string(), true)
        .color(if roundtrip_ms < 100 { 0x00FF00 } else if roundtrip_ms < 250 { 0xFFFF00 } else { 0xFF0000 });

    edit_embed(ctx, cmd, embed).await
}

async fn handle_ask(ctx: &Context, cmd: &CommandInteraction, state: &BotState) -> Result<(), serenity::Error> {
    let ai_service = match state.ai_service.as_ref() {
        Some(s) => s,
        None => {
            return respond_text(ctx, cmd, "AI service not configured").await;
        }
    };
    
    let question = string_option(&cmd.data.options, "question").unwrap_or_default();
    
    cmd.defer(ctx).await?;
    
    let answer = ai_service.generate_response(
        question,
        &cmd.channel_id.to_string(),
        cmd.guild_id.map(|g| g.to_string()).as_deref(),
        &format!("slash_{}", cmd.id),
        &cmd.user.id.to_string(),
        &cmd.user.name,
        None,
    ).await;
    
    match answer {
        Ok(reply) => {
            let question_preview = truncate(question, 1024);
            let answer_preview = truncate(&reply, 1024);
            let embed = CreateEmbed::new()
                .title("🤖 AI Response")
                .field("Question", question_preview, false)
                .field("Answer", answer_preview, false)
                .footer(serenity::builder::CreateEmbedFooter::new(format!("Asked by {}", cmd.user.name)))
                .color(0x5865F2);

            edit_embed(ctx, cmd, embed).await
        }
        Err(e) => edit_text(ctx, cmd, format!("Error: {}", e)).await,
    }
}

async fn handle_stats(ctx: &Context, cmd: &CommandInteraction) -> Result<(), serenity::Error> {
    let Some(guild_id) = cmd.guild_id else {
        return respond_text(ctx, cmd, "This command can only be used in a server").await;
    };

    let (guild_name, total_members, bot_count, channel_count, role_count) = {
        let Some(guild) = ctx.cache.guild(guild_id) else {
            return respond_text(ctx, cmd, "Server data is not in cache yet").await;
        };

        (
            guild.name.clone(),
            guild.member_count,
            guild.members.values().filter(|member| member.user.bot).count(),
            guild.channels.len(),
            guild.roles.len(),
        )
    };

    let human_count = total_members.saturating_sub(bot_count as u64);

    let embed = CreateEmbed::new()
        .title(format!("📊 {}", guild_name))
        .field("Total Members", total_members.to_string(), true)
        .field("Humans", human_count.to_string(), true)
        .field("Bots", bot_count.to_string(), true)
        .field("Channels", channel_count.to_string(), true)
        .field("Roles", role_count.to_string(), true)
        .color(0x5865F2);

    respond_embed(ctx, cmd, embed).await
}

async fn handle_help(ctx: &Context, cmd: &CommandInteraction) -> Result<(), serenity::Error> {
    let embed = CreateEmbed::new()
        .title("📖 Available Commands")
        .description("Here are all the slash commands you can use:")
        .field("/search <query>", "Search UnionCrax for games", false)
        .field("/ask <question>", "Ask the AI a question", false)
        .field("/info", "Show bot information", false)
        .field("/stats", "Display server statistics", false)
        .field("/ping", "Check bot latency", false)
        .field("/location", "Show runtime details", false)
        .field("/birthday", "Manage birthdays and birthday channels", false)
        .field("/settings", "Manage server settings", false)
        .field("/help", "Show this help message", false)
        .color(0x5865F2);
    
    respond_embed(ctx, cmd, embed).await
}

async fn handle_birthday(ctx: &Context, cmd: &CommandInteraction, state: &BotState) -> Result<(), serenity::Error> {
    let Some(option) = cmd.data.options.first() else {
        return respond_text(ctx, cmd, "Use /help to see birthday options").await;
    };

    match option.name.as_str() {
        "set" => {
            let options = sub_options(option);
            let month = int_option(options, "month").unwrap_or_default() as i32;
            let day = int_option(options, "day").unwrap_or_default() as i32;
            let year = int_option(options, "year").map(|value| value as i32);

            if month == 0 || day == 0 {
                return respond_text(ctx, cmd, "Both month and day are required").await;
            }

            let target_user_id = user_option(options, "user").unwrap_or(cmd.user.id);
            let username = if target_user_id == cmd.user.id {
                cmd.user.name.clone()
            } else {
                target_user_id.to_string()
            };

            let success = state
                .database
                .set_birthday(&target_user_id.to_string(), &username, day, month, year)
                .await
                .unwrap_or(false);

            if success {
                respond_text(
                    ctx,
                    cmd,
                    format!(
                        "Birthday set to **{}/{}{}**",
                        month,
                        day,
                        year.map(|value| format!("/{}", value)).unwrap_or_default()
                    ),
                )
                .await
            } else {
                respond_text(ctx, cmd, "Failed to save birthday").await
            }
        }
        "get" => {
            let options = sub_options(option);
            let user_id = user_option(options, "user").unwrap_or(cmd.user.id);

            if let Some(bday) = state.database.get_birthday(&user_id.to_string()).await.unwrap_or(None) {
                respond_text(
                    ctx,
                    cmd,
                    format!(
                        "🎂 Birthday: **{}/{}{}**",
                        bday.month,
                        bday.day,
                        bday.year.map(|value| format!("/{}", value)).unwrap_or_default()
                    ),
                )
                .await
            } else {
                respond_text(ctx, cmd, "No birthday found").await
            }
        }
        "remove" => {
            let options = sub_options(option);
            let user_id = user_option(options, "user").unwrap_or(cmd.user.id);
            let success = state.database.remove_birthday(&user_id.to_string()).await.unwrap_or(false);

            if success {
                respond_text(ctx, cmd, "Birthday removed").await
            } else {
                respond_text(ctx, cmd, "Failed to remove birthday").await
            }
        }
        "test" => {
            let options = sub_options(option);
            let now = chrono::Utc::now();
            let day = int_option(options, "day").unwrap_or(now.day() as i64) as i32;
            let month = int_option(options, "month").unwrap_or(now.month() as i64) as i32;
            let birthdays = state
                .database
                .get_todays_birthdays(day, month, now.year())
                .await
                .unwrap_or_default();

            if birthdays.is_empty() {
                respond_text(ctx, cmd, format!("No birthdays found for {}/{}", month, day)).await
            } else {
                let users = birthdays
                    .iter()
                    .map(|birthday| birthday.username.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                respond_text(
                    ctx,
                    cmd,
                    format!("Birthdays for {}/{}: {}", month, day, users),
                )
                .await
            }
        }
        "send" => {
            let Some(guild_id) = cmd.guild_id else {
                return respond_text(ctx, cmd, "This command can only be used in a server").await;
            };

            let now = chrono::Utc::now();
            let birthdays = state
                .database
                .get_todays_birthdays(now.day() as i32, now.month() as i32, now.year())
                .await
                .unwrap_or_default();

            if birthdays.is_empty() {
                return respond_text(ctx, cmd, "No birthdays to send today").await;
            }

            let target_channel = match state.database.get_birthday_channel(&guild_id.to_string()).await.unwrap_or(None) {
                Some(channel_id) => channel_id
                    .parse::<u64>()
                    .ok()
                    .map(ChannelId::new)
                    .unwrap_or(cmd.channel_id),
                None => cmd.channel_id,
            };

            let mentions = birthdays
                .iter()
                .map(|birthday| format!("<@{}>", birthday.user_id))
                .collect::<Vec<_>>()
                .join(" ");

            target_channel
                .say(ctx, format!("🎉 Happy birthday {}!", mentions))
                .await?;

            for birthday in birthdays {
                let _ = state.database.mark_birthday_as_pinged(&birthday.user_id, now.year()).await;
            }

            respond_text(ctx, cmd, "Birthday announcement sent").await
        }
        "channel" => handle_birthday_channel(ctx, cmd, state, option).await,
        _ => respond_text(ctx, cmd, "Use /help to see birthday options").await,
    }
}

async fn handle_settings(ctx: &Context, cmd: &CommandInteraction, state: &BotState) -> Result<(), serenity::Error> {
    let guild_id = match cmd.guild_id {
        Some(g) => g.to_string(),
        None => {
            return respond_text(ctx, cmd, "This command can only be used in a server").await;
        }
    };
    
    let Some(option) = cmd.data.options.first() else {
        return respond_text(ctx, cmd, "Use /help to see settings options").await;
    };

    match option.name.as_str() {
        "view" => {
            let settings = state.database.get_all_server_settings(&guild_id).await.unwrap_or_default();
            
            let ai_channel = settings.iter()
                .find(|s| s.setting_name == "ai_channel")
                .and_then(|s| s.setting_value.clone())
                .unwrap_or_else(|| "Not set".to_string());
            
            let response_chance = settings.iter()
                .find(|s| s.setting_name == "random_response_chance")
                .and_then(|s| s.setting_value.clone())
                .unwrap_or_else(|| "0.1".to_string());
            
            let embed = CreateEmbed::new()
                .title("⚙️ Server Settings")
                .field("AI Channel", ai_channel, true)
                .field(
                    "Response Chance",
                    format!("{}%", (response_chance.parse::<f64>().unwrap_or(0.1) * 100.0) as i32),
                    true,
                )
                .color(0x5865F2);

            respond_embed(ctx, cmd, embed).await
        }
        "ai_channel" => {
            let options = sub_options(option);
            let channel_id = channel_option(options, "channel").map(|channel| channel.to_string());
            
            if let Some(ch_id) = channel_id {
                state.database.set_server_setting(&guild_id, "ai_channel", &ch_id).await.ok();
                respond_text(ctx, cmd, "AI channel set").await
            } else {
                state.database.remove_server_setting(&guild_id, "ai_channel").await.ok();
                respond_text(ctx, cmd, "AI channel reset").await
            }
        }
        "response_chance" => {
            let options = sub_options(option);
            let Some(percentage) = string_option(options, "percentage") else {
                return respond_text(ctx, cmd, "A percentage value is required").await;
            };

            let chance = match percentage.parse::<f64>() {
                Ok(value) if (0.0..=1.0).contains(&value) => value,
                _ => return respond_text(ctx, cmd, "Percentage must be between 0.0 and 1.0").await,
            };

            state
                .database
                .set_server_setting(&guild_id, "random_response_chance", &chance.to_string())
                .await
                .ok();

            respond_text(ctx, cmd, format!("Response chance set to {:.0}%", chance * 100.0)).await
        }
        _ => respond_text(ctx, cmd, "Use /help to see settings options").await,
    }
}

async fn handle_location(ctx: &Context, cmd: &CommandInteraction) -> Result<(), serenity::Error> {
    let embed = CreateEmbed::new()
        .title("📍 Bot Location Information")
        .field("Platform", std::env::consts::OS, true)
        .field("Architecture", std::env::consts::ARCH, true)
        .field("Process ID", std::process::id().to_string(), true)
        .color(0x0099FF);

    respond_embed(ctx, cmd, embed).await
}

// Message handler
pub async fn handle_message(ctx: Context, msg: Message, state: Arc<BotState>) {
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
        let mentioned = msg.mentions.iter().any(|user| user.id == ctx.cache.current_user().id);
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
    
    if should_respond {
        let Some(ai_service) = state.ai_service.as_ref() else {
            return;
        };
        
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
pub async fn handle_ready(ctx: Context, ready: Ready, _state: Arc<BotState>) {
    tracing::info!("Logged in as {}", ready.user.name);
    
    if let Err(e) = setup_commands(&ctx).await {
        tracing::error!("Failed to setup commands: {}", e);
    }
}

fn build_commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("search")
            .description("Search UnionCrax for games")
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "query", "The game to search for")
                    .required(true),
            ),
        CreateCommand::new("info").description("Get information about the bot"),
        CreateCommand::new("birthday")
            .description("Manage your birthday")
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "set", "Set your birthday")
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::Integer, "month", "Month (1-12)").required(true))
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::Integer, "day", "Day (1-31)").required(true))
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::Integer, "year", "Year (optional)").required(false))
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::User, "user", "User to set birthday for").required(false)),
            )
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "remove", "Remove your birthday")
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::User, "user", "User to remove birthday for").required(false)),
            )
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "get", "Get a birthday")
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::User, "user", "User to get birthday for").required(false)),
            )
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "test", "Test birthday announcements")
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::Integer, "day", "Day to test").required(false))
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::Integer, "month", "Month to test").required(false)),
            )
            .add_option(CreateCommandOption::new(CommandOptionType::SubCommand, "send", "Send today's birthday pings"))
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommandGroup, "channel", "Manage birthday channel")
                    .add_sub_option(
                        CreateCommandOption::new(CommandOptionType::SubCommand, "set", "Set birthday channel")
                            .add_sub_option(CreateCommandOption::new(CommandOptionType::Channel, "channel", "Channel to use").required(true)),
                    )
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::SubCommand, "remove", "Remove birthday channel"))
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::SubCommand, "get", "Get birthday channel")),
            ),
        CreateCommand::new("settings")
            .description("Manage server settings")
            .add_option(CreateCommandOption::new(CommandOptionType::SubCommand, "view", "View all server settings"))
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "ai_channel", "Set AI response channel")
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::Channel, "channel", "Channel for AI responses").required(false)),
            )
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "response_chance", "Set response chance")
                    .add_sub_option(CreateCommandOption::new(CommandOptionType::String, "percentage", "0.0 to 1.0").required(true)),
            ),
        CreateCommand::new("location").description("Get bot location info"),
        CreateCommand::new("ping").description("Check bot latency"),
        CreateCommand::new("ask")
            .description("Ask the AI a question")
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "question", "Your question").required(true),
            ),
        CreateCommand::new("stats").description("Show server statistics"),
        CreateCommand::new("help").description("Show available commands"),
    ]
}

async fn handle_birthday_channel(
    ctx: &Context,
    cmd: &CommandInteraction,
    state: &BotState,
    option: &CommandDataOption,
) -> Result<(), serenity::Error> {
    let Some(guild_id) = cmd.guild_id else {
        return respond_text(ctx, cmd, "This command can only be used in a server").await;
    };

    let Some(channel_option_group) = sub_options(option).first() else {
        return respond_text(ctx, cmd, "Use /birthday channel with set, get, or remove").await;
    };

    match channel_option_group.name.as_str() {
        "set" => {
            let options = sub_options(channel_option_group);
            let Some(channel_id) = channel_option(options, "channel") else {
                return respond_text(ctx, cmd, "A channel is required").await;
            };

            let success = state
                .database
                .set_birthday_channel(&guild_id.to_string(), &channel_id.to_string())
                .await
                .unwrap_or(false);

            if success {
                respond_text(ctx, cmd, format!("Birthday channel set to <#{}>", channel_id)).await
            } else {
                respond_text(ctx, cmd, "Failed to set birthday channel").await
            }
        }
        "get" => match state.database.get_birthday_channel(&guild_id.to_string()).await.unwrap_or(None) {
            Some(channel_id) => respond_text(ctx, cmd, format!("Birthday channel: <#{}>", channel_id)).await,
            None => respond_text(ctx, cmd, "No birthday channel configured").await,
        },
        "remove" => {
            let success = state.database.remove_birthday_channel(&guild_id.to_string()).await.unwrap_or(false);

            if success {
                respond_text(ctx, cmd, "Birthday channel removed").await
            } else {
                respond_text(ctx, cmd, "Failed to remove birthday channel").await
            }
        }
        _ => respond_text(ctx, cmd, "Use /birthday channel with set, get, or remove").await,
    }
}

async fn respond_text(
    ctx: &Context,
    cmd: &CommandInteraction,
    content: impl Into<String>,
) -> Result<(), serenity::Error> {
    cmd.create_response(
        ctx,
        CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().content(content)),
    )
    .await
}

async fn respond_embed(
    ctx: &Context,
    cmd: &CommandInteraction,
    embed: CreateEmbed,
) -> Result<(), serenity::Error> {
    cmd.create_response(
        ctx,
        CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().embed(embed)),
    )
    .await
}

async fn edit_text(
    ctx: &Context,
    cmd: &CommandInteraction,
    content: impl Into<String>,
) -> Result<(), serenity::Error> {
    cmd.edit_response(ctx, EditInteractionResponse::new().content(content))
        .await
        .map(|_| ())
}

async fn edit_embed(
    ctx: &Context,
    cmd: &CommandInteraction,
    embed: CreateEmbed,
) -> Result<(), serenity::Error> {
    cmd.edit_response(ctx, EditInteractionResponse::new().embed(embed))
        .await
        .map(|_| ())
}

fn sub_options(option: &CommandDataOption) -> &[CommandDataOption] {
    match &option.value {
        CommandDataOptionValue::SubCommand(options)
        | CommandDataOptionValue::SubCommandGroup(options) => options.as_slice(),
        _ => &[],
    }
}

fn find_option<'a>(options: &'a [CommandDataOption], name: &str) -> Option<&'a CommandDataOption> {
    options.iter().find(|option| option.name == name)
}

fn string_option<'a>(options: &'a [CommandDataOption], name: &str) -> Option<&'a str> {
    find_option(options, name)?.value.as_str()
}

fn int_option(options: &[CommandDataOption], name: &str) -> Option<i64> {
    find_option(options, name)?.value.as_i64()
}

fn user_option(options: &[CommandDataOption], name: &str) -> Option<UserId> {
    find_option(options, name)?.value.as_user_id()
}

fn channel_option(options: &[CommandDataOption], name: &str) -> Option<ChannelId> {
    find_option(options, name)?.value.as_channel_id()
}

fn truncate(value: &str, max_len: usize) -> String {
    value.chars().take(max_len).collect()
}
