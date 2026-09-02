CREATE EXTENSION IF NOT EXISTS pgcrypto;

ALTER TABLE api_tokens ADD COLUMN id BIGSERIAL;
ALTER TABLE api_tokens ADD COLUMN token_hash BYTEA;
ALTER TABLE api_tokens ADD COLUMN valid_from TIMESTAMPTZ;
ALTER TABLE api_tokens ADD COLUMN valid_to TIMESTAMPTZ;
ALTER TABLE api_tokens ADD COLUMN last_used_at TIMESTAMPTZ;

UPDATE api_tokens
SET token_hash = digest(token, 'sha256');

ALTER TABLE api_tokens DROP CONSTRAINT api_tokens_pkey;
ALTER TABLE api_tokens DROP COLUMN token;
ALTER TABLE api_tokens ALTER COLUMN token_hash SET NOT NULL;
ALTER TABLE api_tokens ADD PRIMARY KEY (id);
ALTER TABLE api_tokens ADD UNIQUE (token_hash);
ALTER TABLE api_tokens ADD CONSTRAINT api_tokens_valid_range
    CHECK (valid_from IS NULL OR valid_to IS NULL OR valid_from <= valid_to);
