ALTER TABLE messages
    ADD COLUMN archive_incomplete BOOLEAN NOT NULL DEFAULT FALSE;

DROP INDEX message_versions_latest_idx;
DROP INDEX attachments_message_version_idx;
DROP INDEX embeds_message_version_idx;
