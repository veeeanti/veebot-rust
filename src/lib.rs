//! veebot - A Discord bot for UnionCrax
//!
//! This is the main library module that exports all components of the bot.

pub mod ai;
pub mod bot;
pub mod config;
pub mod context;
pub mod database;
pub mod embeddings;
pub mod postgres;
pub mod search;
pub mod sqlite;

pub use config::Config;
pub use database::DatabaseManager;
pub use context::ContextManager;
pub use ai::AiService;
pub use search::SearchService;
