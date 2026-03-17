-- SQLite Schema for UC-AIv2
-- This schema is designed for local hosting using a single file.

-- Messages table
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    discord_message_id TEXT UNIQUE,
    content TEXT NOT NULL,
    author_id TEXT NOT NULL,
    author_name TEXT NOT NULL,
    message_type TEXT DEFAULT 'user',
    guild_id VARCHAR(255),
    channel_id VARCHAR(255),
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Indexes for performance
CREATE INDEX IF NOT EXISTS idx_messages_channel_id ON messages(channel_id);
CREATE INDEX IF NOT EXISTS idx_messages_guild_id ON messages(guild_id);
CREATE INDEX IF NOT EXISTS idx_messages_created_at ON messages(created_at);
CREATE INDEX IF NOT EXISTS idx_messages_message_type ON messages(message_type);

-- Full-text search table for messages
-- We use a virtual FTS5 table to index the content for fast searching.
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    content,
    content='messages',
    content_rowid='id'
);

-- Triggers to keep the FTS index in sync with the messages table
CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
END;

CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
  INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;

-- Memories table for explicit user memories
CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    username TEXT NOT NULL,
    guild_id VARCHAR(255),
    channel_id VARCHAR(255),
    memory TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Indexes for memories
CREATE INDEX IF NOT EXISTS idx_memories_user_id ON memories(user_id);
CREATE INDEX IF NOT EXISTS idx_memories_guild_id ON memories(guild_id);
CREATE INDEX IF NOT EXISTS idx_memories_channel_id ON memories(channel_id);
CREATE INDEX IF NOT EXISTS idx_memories_created_at ON memories(created_at);

-- Full-text search table for memories
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    memory,
    content='memories',
    content_rowid='id'
);

-- Triggers to keep the FTS index in sync with the memories table
CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
  INSERT INTO memories_fts(rowid, memory) VALUES (new.id, new.memory);
END;

CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, memory) VALUES('delete', old.id, old.memory);
END;

CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, memory) VALUES('delete', old.id, old.memory);
  INSERT INTO memories_fts(rowid, memory) VALUES (new.id, new.memory);
END;

-- Trigger to automatically update updated_at on memories
CREATE TRIGGER IF NOT EXISTS update_memories_updated_at AFTER UPDATE ON memories
BEGIN
    UPDATE memories SET updated_at = CURRENT_TIMESTAMP WHERE id = old.id;
END;

-- Birthdays table
CREATE TABLE IF NOT EXISTS birthdays (
    user_id TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    day INTEGER NOT NULL,
    month INTEGER NOT NULL,
    year INTEGER,
    last_pinged_year INTEGER DEFAULT 0,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Trigger to automatically update updated_at on messages
CREATE TRIGGER IF NOT EXISTS update_messages_updated_at AFTER UPDATE ON messages
BEGIN
    UPDATE messages SET updated_at = CURRENT_TIMESTAMP WHERE id = old.id;
END;

-- Trigger to automatically update updated_at on birthdays
CREATE TRIGGER IF NOT EXISTS update_birthdays_updated_at AFTER UPDATE ON birthdays
BEGIN
    UPDATE birthdays SET updated_at = CURRENT_TIMESTAMP WHERE user_id = old.user_id;
END;

-- Birthday channels table (per-guild birthday announcement channel)
CREATE TABLE IF NOT EXISTS birthday_channels (
    guild_id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Trigger to automatically update updated_at on birthday_channels
CREATE TRIGGER IF NOT EXISTS update_birthday_channels_updated_at AFTER UPDATE ON birthday_channels
BEGIN
    UPDATE birthday_channels SET updated_at = CURRENT_TIMESTAMP WHERE guild_id = old.guild_id;
END;

-- Autoban channels table (per-guild channel where spam detection is active)
CREATE TABLE IF NOT EXISTS autoban_channels (
    guild_id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Trigger to automatically update updated_at on autoban_channels
CREATE TRIGGER IF NOT EXISTS update_autoban_channels_updated_at AFTER UPDATE ON autoban_channels
BEGIN
    UPDATE autoban_channels SET updated_at = CURRENT_TIMESTAMP WHERE guild_id = old.guild_id;
END;

-- Server settings table (per-guild configuration)
CREATE TABLE IF NOT EXISTS server_settings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id VARCHAR(255) NOT NULL,
    setting_name VARCHAR(100) NOT NULL,
    setting_value TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(guild_id, setting_name)
);

-- Indexes for server_settings
CREATE INDEX IF NOT EXISTS idx_server_settings_guild_id ON server_settings(guild_id);

-- Trigger to automatically update updated_at on server_settings
CREATE TRIGGER IF NOT EXISTS update_server_settings_updated_at AFTER UPDATE ON server_settings
BEGIN
    UPDATE server_settings SET updated_at = CURRENT_TIMESTAMP WHERE id = old.id;
END;
