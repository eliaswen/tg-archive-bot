CREATE TABLE guild_history (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    guild_id BIGINT NOT NULL,
    guild_name TEXT NOT NULL,
    guild_icon_url TEXT,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE NULLS NOT DISTINCT (guild_id, guild_name, guild_icon_url),

    FOREIGN KEY (guild_id)
        REFERENCES guilds(guild_id)
        ON DELETE CASCADE
);

CREATE TABLE channel_history (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    channel_id BIGINT NOT NULL,
    channel_name TEXT NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (channel_id, channel_name),

    FOREIGN KEY (channel_id)
        REFERENCES channels(channel_id)
        ON DELETE CASCADE
);

CREATE TABLE discord_user_history (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    discord_id BIGINT NOT NULL,
    discord_username TEXT NOT NULL,
    discord_avatar_url TEXT,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE NULLS NOT DISTINCT (discord_id, discord_username, discord_avatar_url),

    FOREIGN KEY (discord_id)
        REFERENCES discord_users(discord_id)
        ON DELETE CASCADE
);

CREATE TABLE guild_users (
    guild_id BIGINT NOT NULL,
    discord_id BIGINT NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (guild_id, discord_id),

    FOREIGN KEY (guild_id)
        REFERENCES guilds(guild_id)
        ON DELETE CASCADE,

    FOREIGN KEY (discord_id)
        REFERENCES discord_users(discord_id)
        ON DELETE CASCADE
);

INSERT INTO guild_history (guild_id, guild_name, guild_icon_url)
SELECT guild_id, guild_name, guild_icon_url
FROM guilds;

INSERT INTO channel_history (channel_id, channel_name)
SELECT channel_id, channel_name
FROM channels;

INSERT INTO discord_user_history (discord_id, discord_username, discord_avatar_url)
SELECT discord_id, discord_username, NULL
FROM discord_users;

INSERT INTO guild_users (guild_id, discord_id)
SELECT guild_id, author_id
FROM messages
GROUP BY guild_id, author_id;
