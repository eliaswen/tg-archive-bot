CREATE TABLE IF NOT EXISTS guilds (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    guild_id BIGINT NOT NULL UNIQUE,
    guild_name TEXT NOT NULL,
    guild_icon_url TEXT
);


CREATE TABLE IF NOT EXISTS channels (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL UNIQUE,
    channel_name TEXT NOT NULL,

    FOREIGN KEY (guild_id)
        REFERENCES guilds(guild_id)
);


CREATE TABLE IF NOT EXISTS discord_users (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    discord_id BIGINT NOT NULL UNIQUE,
    discord_username TEXT NOT NULL
);


CREATE TABLE IF NOT EXISTS discord_roles (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    guild_id BIGINT NOT NULL,
    discord_id BIGINT NOT NULL,
    discord_role_name TEXT NOT NULL,

    UNIQUE (guild_id, discord_id),

    FOREIGN KEY (guild_id)
        REFERENCES guilds(guild_id)
);


CREATE TABLE IF NOT EXISTS messages (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    message_id BIGINT NOT NULL UNIQUE,

    author_id BIGINT NOT NULL,

    author_username TEXT NOT NULL,

    content TEXT,

    timestamp TIMESTAMPTZ NOT NULL,

    FOREIGN KEY (guild_id)
        REFERENCES guilds(guild_id),

    FOREIGN KEY (channel_id)
        REFERENCES channels(channel_id),

    FOREIGN KEY (author_id)
        REFERENCES discord_users(discord_id)
);


CREATE TABLE IF NOT EXISTS attachments (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    attachment_id BIGINT NOT NULL UNIQUE,
    message_id BIGINT NOT NULL,

    filename TEXT NOT NULL,
    description TEXT,
    content_type TEXT,
    size BIGINT NOT NULL,

    width INTEGER,
    height INTEGER,

    data BYTEA NOT NULL,

    FOREIGN KEY (message_id)
        REFERENCES messages(message_id)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS embeds (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    message_id BIGINT NOT NULL,
    embed_index INTEGER NOT NULL,

    embed_type TEXT,
    title TEXT,
    description TEXT,
    url TEXT,
    timestamp TIMESTAMPTZ,
    color INTEGER,

    footer_text TEXT,
    footer_icon_url TEXT,

    image_url TEXT,
    image_proxy_url TEXT,
    image_width INTEGER,
    image_height INTEGER,

    thumbnail_url TEXT,
    thumbnail_proxy_url TEXT,
    thumbnail_width INTEGER,
    thumbnail_height INTEGER,

    video_url TEXT,
    video_proxy_url TEXT,
    video_width INTEGER,
    video_height INTEGER,

    provider_name TEXT,
    provider_url TEXT,

    author_name TEXT,
    author_url TEXT,
    author_icon_url TEXT,
    author_proxy_icon_url TEXT,

    UNIQUE (message_id, embed_index),

    FOREIGN KEY (message_id)
        REFERENCES messages(message_id)
        ON DELETE CASCADE
);


CREATE TABLE IF NOT EXISTS embed_fields (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    embed_uuid UUID NOT NULL,
    field_index INTEGER NOT NULL,

    name TEXT NOT NULL,
    value TEXT NOT NULL,
    inline BOOLEAN NOT NULL DEFAULT FALSE,

    UNIQUE (embed_uuid, field_index),

    FOREIGN KEY (embed_uuid)
        REFERENCES embeds(uuid)
        ON DELETE CASCADE
);
