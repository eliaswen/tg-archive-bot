use sqlx::PgPool;

pub struct ArchiveStats {
    pub messages: i64,
    pub users: i64,
    pub servers: i64,
    pub channels: i64,
    pub total_storage: i64,
    pub message_storage: i64,
    pub attachment_storage: i64,
}

pub async fn load(pool: &PgPool) -> Result<ArchiveStats, sqlx::Error> {
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64)>(
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
              FROM attachments);",
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
