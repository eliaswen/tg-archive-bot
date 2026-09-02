CREATE INDEX archive_queue_message_order_idx
    ON archive_queue (message_id, id)
    WHERE failed_at IS NULL;
