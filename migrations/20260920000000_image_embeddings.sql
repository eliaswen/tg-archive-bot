CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE image_embeddings (
    message_id BIGINT NOT NULL,
    message_version BIGINT NOT NULL,
    attachment_id BIGINT NOT NULL,
    embedding vector(768) NOT NULL,
    model_revision TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (message_id, message_version, attachment_id),

    FOREIGN KEY (message_id, message_version, attachment_id)
        REFERENCES attachments(message_id, message_version, attachment_id)
        ON DELETE CASCADE
);

CREATE TABLE image_embedding_jobs (
    message_id BIGINT NOT NULL,
    message_version BIGINT NOT NULL,
    attachment_id BIGINT NOT NULL,
    desired_model_revision TEXT NOT NULL DEFAULT 'onnx-community/siglip2-base-patch16-224-ONNX@ba1f3b0843f24bc5417d38e19c37b287d719b2f4',
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    locked_until TIMESTAMPTZ,
    last_error TEXT,
    failed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (message_id, message_version, attachment_id),

    FOREIGN KEY (message_id, message_version, attachment_id)
        REFERENCES attachments(message_id, message_version, attachment_id)
        ON DELETE CASCADE
);

INSERT INTO image_embedding_jobs (message_id, message_version, attachment_id)
SELECT a.message_id, a.message_version, a.attachment_id
FROM attachments a
LEFT JOIN image_embeddings e
  ON e.message_id = a.message_id
 AND e.message_version = a.message_version
 AND e.attachment_id = a.attachment_id
WHERE e.attachment_id IS NULL
  AND (a.content_type IN ('image/jpeg', 'image/jpg', 'image/png', 'image/gif', 'image/webp', 'image/bmp', 'image/x-ms-bmp') OR ((a.width IS NOT NULL AND a.height IS NOT NULL) AND a.filename ~* '\.(jpe?g|png|gif|webp|bmp)$'));

CREATE INDEX image_embedding_jobs_ready_idx
    ON image_embedding_jobs (available_at, created_at)
    WHERE failed_at IS NULL;
