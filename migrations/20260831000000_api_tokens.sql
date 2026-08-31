CREATE TABLE IF NOT EXISTS api_tokens (
    token TEXT PRIMARY KEY,
    discord_id BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    FOREIGN KEY (discord_id)
        REFERENCES discord_users(discord_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS api_tokens_discord_id_idx
    ON api_tokens(discord_id);
