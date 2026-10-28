use super::*;

pub(in crate::web) async fn channel(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(channel_id): Path<i64>,
) -> WebResult {
    let channel = sqlx::query_as::<_, (String, i64, String)>(
        "SELECT c.channel_name, g.guild_id, g.guild_name
         FROM channels c
         JOIN guilds g ON g.guild_id = c.guild_id
         WHERE c.channel_id = $1;",
    )
    .bind(channel_id)
    .fetch_optional(&data.pool)
    .await
    .map_err(database_error)?
    .ok_or_else(|| not_found("Channel not found."))?;
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(
        "SELECT
             (SELECT COUNT(*) FROM messages WHERE channel_id = $1),
             (SELECT COUNT(DISTINCT author_id) FROM messages WHERE channel_id = $1),
             (SELECT COUNT(*)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.channel_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(v.content)), 0)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.channel_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(a.data)), 0)
              FROM attachments a
              JOIN messages m ON m.message_id = a.message_id
              WHERE m.channel_id = $1);",
    )
    .bind(channel_id)
    .fetch_one(&data.pool)
    .await
    .map_err(database_error)?;
    let names = sqlx::query_as::<_, (String, String, String)>(
        "SELECT channel_name, first_seen_at::text, last_seen_at::text
         FROM channel_history
         WHERE channel_id = $1
         ORDER BY first_seen_at, channel_name;",
    )
    .bind(channel_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let users = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT u.discord_id, u.discord_username, COUNT(m.message_id)
         FROM messages m
         JOIN discord_users u ON u.discord_id = m.author_id
         WHERE m.channel_id = $1
         GROUP BY u.discord_id, u.discord_username
         ORDER BY u.discord_username, u.discord_id;",
    )
    .bind(channel_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;

    let body = render_template(&ChannelStatusTemplate {
        channel_name: &channel.0,
        channel_id,
        server_id: channel.1,
        server_name: &channel.2,
        message_count: stats.0,
        user_count: stats.1,
        version_count: stats.2,
        total_storage: format_bytes(stats.3 + stats.4),
        message_storage: format_bytes(stats.3),
        attachment_storage: format_bytes(stats.4),
        names: render_status_names(names),
        users: render_status_users(users),
        access_reason: user.access_reason(channel_id),
    });
    Ok(Html(page(&channel.0, &body)))
}
