ALTER TABLE discord_user_history
    ADD COLUMN image_data BYTEA,
    ADD COLUMN image_available BOOLEAN,
    ADD COLUMN image_checked BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN image_mimetype TEXT;