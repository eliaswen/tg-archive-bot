CREATE TABLE archive_queue (
    id BIGSERIAL PRIMARY KEY,
    message_id BIGINT NOT NULL,
    is_edit BOOLEAN NOT NULL,
    payload JSONB NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    locked_until TIMESTAMPTZ,
    last_error TEXT,
    failed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE message_versions
    ADD COLUMN source_event_id BIGINT UNIQUE;

CREATE INDEX archive_queue_ready_idx
    ON archive_queue (available_at, id)
    WHERE failed_at IS NULL;
CREATE INDEX messages_channel_timeline_idx
    ON messages (channel_id, timestamp DESC, message_id DESC);
CREATE INDEX messages_author_timeline_idx
    ON messages (author_id, timestamp DESC, message_id DESC);
CREATE INDEX messages_guild_timeline_idx
    ON messages (guild_id, timestamp DESC, message_id DESC);
CREATE INDEX message_versions_latest_idx
    ON message_versions (message_id, version DESC);
CREATE INDEX attachments_message_version_idx
    ON attachments (message_id, message_version);
CREATE INDEX embeds_message_version_idx
    ON embeds (message_id, message_version);
CREATE INDEX guild_users_discord_id_idx
    ON guild_users (discord_id, guild_id);

CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE INDEX messages_content_trgm_idx
    ON messages USING GIN ((COALESCE(content, '')) gin_trgm_ops);
CREATE INDEX discord_users_username_trgm_idx
    ON discord_users USING GIN (discord_username gin_trgm_ops);
