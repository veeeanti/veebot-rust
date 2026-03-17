-- PostgreSQL Schema for UC-AIv2 Context Management

-- Messages table without embeddings
CREATE TABLE IF NOT EXISTS messages (
    id BIGSERIAL PRIMARY KEY,
    discord_message_id VARCHAR(255),
    content TEXT NOT NULL,
    author_id VARCHAR(255) NOT NULL,
    author_name VARCHAR(255) NOT NULL,
    channel_id VARCHAR(255) NOT NULL,
    guild_id VARCHAR(255),
    message_type VARCHAR(50) DEFAULT 'user',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Indexes for performance
CREATE INDEX IF NOT EXISTS idx_messages_channel_id ON messages(channel_id);
CREATE INDEX IF NOT EXISTS idx_messages_guild_id ON messages(guild_id);
CREATE INDEX IF NOT EXISTS idx_messages_created_at ON messages(created_at);
CREATE INDEX IF NOT EXISTS idx_messages_message_type ON messages(message_type);

-- Full-text search index for content-based search
CREATE INDEX IF NOT EXISTS idx_messages_content ON messages USING gin(to_tsvector('english', content));

-- Function to update updated_at timestamp
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

-- Trigger to automatically update updated_at
DROP TRIGGER IF EXISTS update_messages_updated_at ON messages;
CREATE TRIGGER update_messages_updated_at
    BEFORE UPDATE ON messages
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Memories table for explicit user memories
CREATE TABLE IF NOT EXISTS memories (
    id BIGSERIAL PRIMARY KEY,
    user_id VARCHAR(255) NOT NULL,
    username VARCHAR(255) NOT NULL,
    memory TEXT NOT NULL,
    guild_id VARCHAR(255),
    channel_id VARCHAR(255),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Indexes for memories
CREATE INDEX IF NOT EXISTS idx_memories_user_id ON memories(user_id);
CREATE INDEX IF NOT EXISTS idx_memories_guild_id ON memories(guild_id);
CREATE INDEX IF NOT EXISTS idx_memories_channel_id ON memories(channel_id);
CREATE INDEX IF NOT EXISTS idx_memories_created_at ON memories(created_at);

-- Full-text search index for memories
CREATE INDEX IF NOT EXISTS idx_memories_memory ON memories USING gin(to_tsvector('english', memory));

-- Trigger for memories
DROP TRIGGER IF EXISTS update_memories_updated_at ON memories;
CREATE TRIGGER update_memories_updated_at
    BEFORE UPDATE ON memories
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Birthdays table
CREATE TABLE IF NOT EXISTS birthdays (
    user_id VARCHAR(255) PRIMARY KEY,
    username VARCHAR(255) NOT NULL,
    day INTEGER NOT NULL,
    month INTEGER NOT NULL,
    year INTEGER,
    last_pinged_year INTEGER DEFAULT 0,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Trigger for birthdays
DROP TRIGGER IF EXISTS update_birthdays_updated_at ON birthdays;
CREATE TRIGGER update_birthdays_updated_at
    BEFORE UPDATE ON birthdays
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Birthday channels table (per-guild birthday announcement channel)
CREATE TABLE IF NOT EXISTS birthday_channels (
    guild_id VARCHAR(255) PRIMARY KEY,
    channel_id VARCHAR(255) NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Trigger for birthday_channels
DROP TRIGGER IF EXISTS update_birthday_channels_updated_at ON birthday_channels;
CREATE TRIGGER update_birthday_channels_updated_at
    BEFORE UPDATE ON birthday_channels
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Autoban channels table (per-guild channel where spam detection is active)
CREATE TABLE IF NOT EXISTS autoban_channels (
    guild_id VARCHAR(255) PRIMARY KEY,
    channel_id VARCHAR(255) NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- Trigger for autoban_channels
DROP TRIGGER IF EXISTS update_autoban_channels_updated_at ON autoban_channels;
CREATE TRIGGER update_autoban_channels_updated_at
    BEFORE UPDATE ON autoban_channels
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Server settings table (per-guild configuration)
CREATE TABLE IF NOT EXISTS server_settings (
    id BIGSERIAL PRIMARY KEY,
    guild_id VARCHAR(255) NOT NULL,
    setting_name VARCHAR(100) NOT NULL,
    setting_value TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(guild_id, setting_name)
);

-- Indexes for server_settings
CREATE INDEX IF NOT EXISTS idx_server_settings_guild_id ON server_settings(guild_id);

-- Trigger for server_settings
DROP TRIGGER IF EXISTS update_server_settings_updated_at ON server_settings;
CREATE TRIGGER update_server_settings_updated_at
    BEFORE UPDATE ON server_settings
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();