CREATE TABLE message_versions (
    uuid UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    message_id BIGINT NOT NULL,
    version BIGINT NOT NULL,
    content TEXT,
    archived_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (message_id, version),

    FOREIGN KEY (message_id)
        REFERENCES messages(message_id)
        ON DELETE CASCADE
);

INSERT INTO message_versions (message_id, version, content)
SELECT message_id, 1, content
FROM messages;

ALTER TABLE attachments
    ADD COLUMN message_version BIGINT NOT NULL DEFAULT 1;

ALTER TABLE attachments
    DROP CONSTRAINT attachments_attachment_id_key;

ALTER TABLE attachments
    ADD UNIQUE (message_id, message_version, attachment_id),
    ADD FOREIGN KEY (message_id, message_version)
        REFERENCES message_versions(message_id, version)
        ON DELETE CASCADE;

ALTER TABLE embeds
    ADD COLUMN message_version BIGINT NOT NULL DEFAULT 1;

ALTER TABLE embeds
    DROP CONSTRAINT embeds_message_id_embed_index_key;

ALTER TABLE embeds
    ADD UNIQUE (message_id, message_version, embed_index),
    ADD FOREIGN KEY (message_id, message_version)
        REFERENCES message_versions(message_id, version)
        ON DELETE CASCADE;
