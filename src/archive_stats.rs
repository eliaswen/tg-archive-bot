use sqlx::PgPool;

pub struct ArchiveStats {
    pub messages: i64,
    pub users: i64,
    pub servers: i64,
    pub channels: i64,
    pub total_storage: i64,
    pub message_storage: i64,
    pub attachment_storage: i64,
    pub processed_images: i64,
    pub unprocessed_images: i64,
    pub average_image_processing_ms: Option<f64>,
    pub average_daily_storage: i64,
    pub running_seconds: i64,
}

pub async fn load(pool: &PgPool) -> Result<ArchiveStats, sqlx::Error> {
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64, i64, i64, Option<f64>, i64, i64)>(
        "SELECT
             (SELECT COUNT(*) FROM messages),
             (SELECT COUNT(DISTINCT author_id) FROM messages),
             (SELECT COUNT(*) FROM guilds),
             (SELECT COUNT(*) FROM channels),
             (SELECT COALESCE(SUM(OCTET_LENGTH(content)), 0)
              FROM message_versions)
                 + (SELECT COALESCE(SUM(OCTET_LENGTH(data)), 0)
                    FROM attachments),
             (SELECT COALESCE(SUM(OCTET_LENGTH(content)), 0)
              FROM message_versions),
             (SELECT COALESCE(SUM(OCTET_LENGTH(data)), 0)
              FROM attachments),
             (SELECT COUNT(*) FROM image_embeddings),
             (SELECT COUNT(*)
              FROM attachments a
              LEFT JOIN image_embeddings e
                ON e.message_id = a.message_id
               AND e.message_version = a.message_version
               AND e.attachment_id = a.attachment_id
              WHERE e.attachment_id IS NULL
                AND (a.content_type IN ('image/jpeg', 'image/jpg', 'image/png', 'image/gif', 'image/webp', 'image/bmp', 'image/x-ms-bmp') OR ((a.width IS NOT NULL AND a.height IS NOT NULL) AND a.filename ~* '\\.(jpe?g|png|gif|webp|bmp)$'))),
             (SELECT AVG(processing_duration_ms)::DOUBLE PRECISION
              FROM image_embeddings
              WHERE processing_duration_ms IS NOT NULL),
             CASE WHEN (SELECT MIN(archived_at) FROM message_versions) IS NULL THEN 0 ELSE
                 ((SELECT COALESCE(SUM(OCTET_LENGTH(content)), 0) FROM message_versions)
                  + (SELECT COALESCE(SUM(OCTET_LENGTH(data)), 0) FROM attachments))
                 / GREATEST(CURRENT_DATE - (SELECT MIN(archived_at)::date FROM message_versions) + 1, 1)
             END,
             COALESCE(EXTRACT(EPOCH FROM NOW() - (SELECT MIN(archived_at) FROM message_versions))::BIGINT, 0);",
    )
    .fetch_one(pool)
    .await?;
    Ok(ArchiveStats {
        messages: stats.0,
        users: stats.1,
        servers: stats.2,
        channels: stats.3,
        total_storage: stats.4,
        message_storage: stats.5,
        attachment_storage: stats.6,
        processed_images: stats.7,
        unprocessed_images: stats.8,
        average_image_processing_ms: stats.9,
        average_daily_storage: stats.10,
        running_seconds: stats.11,
    })
}

pub fn format_bytes(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];

    let mut size = bytes.max(0) as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", bytes.max(0), UNITS[unit])
    } else {
        format!("{:.1} {}", size, UNITS[unit])
    }
}

pub fn format_duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    let seconds = seconds % 60;
    let day_unit = if days == 1 { "day" } else { "days" };
    let hour_unit = if hours == 1 { "hour" } else { "hours" };
    let minute_unit = if minutes == 1 { "minute" } else { "minutes" };
    let second_unit = if seconds == 1 { "second" } else { "seconds" };

    if days > 0 {
        format!(
            "{days} {day_unit}, {hours} {hour_unit}, {minutes} {minute_unit}, {seconds} {second_unit}"
        )
    } else if hours > 0 {
        format!("{hours} {hour_unit}, {minutes} {minute_unit}, {seconds} {second_unit}")
    } else if minutes > 0 {
        format!("{minutes} {minute_unit}, {seconds} {second_unit}")
    } else {
        format!("{seconds} {second_unit}")
    }
}

pub fn format_processing_speed(milliseconds: Option<f64>) -> String {
    match milliseconds {
        Some(milliseconds) if milliseconds >= 1_000.0 => {
            format!("{:.2} seconds per image", milliseconds / 1_000.0)
        }
        Some(milliseconds) => format!("{milliseconds:.0} ms per image"),
        None => "Not available yet".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_archive_running_time() {
        assert_eq!(format_duration(45), "45 seconds");
        assert_eq!(format_duration(3_725), "1 hour, 2 minutes, 5 seconds");
        assert_eq!(
            format_duration(93_784),
            "1 day, 2 hours, 3 minutes, 4 seconds"
        );
    }

    #[test]
    fn formats_image_processing_speed() {
        assert_eq!(format_processing_speed(None), "Not available yet");
        assert_eq!(format_processing_speed(Some(325.4)), "325 ms per image");
        assert_eq!(
            format_processing_speed(Some(1_250.0)),
            "1.25 seconds per image"
        );
    }
}
