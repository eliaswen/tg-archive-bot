use super::*;

pub(in crate::web) async fn user(
    State(data): State<WebData>,
    Extension(web_user): Extension<WebUser>,
    Path(user_id): Path<i64>,
) -> WebResult {
    let username = sqlx::query_scalar::<_, String>(
        "SELECT discord_username FROM discord_users WHERE discord_id = $1;",
    )
    .bind(user_id)
    .fetch_optional(&data.pool)
    .await
    .map_err(database_error)?
    .ok_or_else(|| not_found("User not found."))?;
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(
        "SELECT
             (SELECT COUNT(*) FROM messages WHERE author_id = $1),
             (SELECT COUNT(*) FROM guild_users WHERE discord_id = $1),
             (SELECT COUNT(DISTINCT channel_id) FROM messages WHERE author_id = $1),
             (SELECT COUNT(*)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.author_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(v.content)), 0)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.author_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(a.data)), 0)
              FROM attachments a
              JOIN messages m ON m.message_id = a.message_id
              WHERE m.author_id = $1);",
    )
    .bind(user_id)
    .fetch_one(&data.pool)
    .await
    .map_err(database_error)?;
    let names = sqlx::query_as::<_, (String, String, String)>(
        "SELECT discord_username, MIN(first_seen_at)::text, MAX(last_seen_at)::text
         FROM discord_user_history
         WHERE discord_id = $1
         GROUP BY discord_username
         ORDER BY MIN(first_seen_at), discord_username;",
    )
    .bind(user_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let avatars = sqlx::query_as::<_, (String, String, bool, Option<bool>, String, String)>(
        "SELECT uuid, discord_avatar_url, image_checked, image_available, first_seen::text, last_seen::text
         FROM (
             SELECT DISTINCT ON (discord_avatar_url)
                 uuid::text, discord_avatar_url, image_checked, image_available,
                 MIN(first_seen_at) OVER (PARTITION BY discord_avatar_url) AS first_seen,
                 MAX(last_seen_at) OVER (PARTITION BY discord_avatar_url) AS last_seen
             FROM discord_user_history
             WHERE discord_id = $1 AND discord_avatar_url IS NOT NULL
             ORDER BY discord_avatar_url, image_checked DESC, image_available DESC, last_seen_at DESC
         ) avatars
         ORDER BY first_seen, discord_avatar_url;",
    )
    .bind(user_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let servers = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT g.guild_id, g.guild_name, COUNT(m.message_id)
         FROM guild_users gu
         JOIN guilds g ON g.guild_id = gu.guild_id
         JOIN messages m ON m.guild_id = gu.guild_id AND m.author_id = gu.discord_id
         WHERE gu.discord_id = $1 AND m.channel_id = ANY($2)
         GROUP BY g.guild_id, g.guild_name
         ORDER BY g.guild_name, g.guild_id;",
    )
    .bind(user_id)
    .bind(&web_user.channel_ids)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let channels = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT c.channel_id, c.channel_name, COUNT(m.message_id)
         FROM messages m
         JOIN channels c ON c.channel_id = m.channel_id
         WHERE m.author_id = $1 AND m.channel_id = ANY($2)
         GROUP BY c.channel_id, c.channel_name
         ORDER BY c.channel_name, c.channel_id;",
    )
    .bind(user_id)
    .bind(&web_user.channel_ids)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let access_reason = channels
        .first()
        .map(|channel| web_user.access_reason(channel.0))
        .unwrap_or("you can see archived messages from them");

    let body = render_template(&UserStatusTemplate {
        username: &username,
        user_id,
        message_count: stats.0,
        server_count: stats.1,
        channel_count: stats.2,
        version_count: stats.3,
        total_storage: format_bytes(stats.4 + stats.5),
        message_storage: format_bytes(stats.4),
        attachment_storage: format_bytes(stats.5),
        names: render_status_names(names),
        avatars: render_status_avatars(avatars),
        servers: render_status_servers(servers),
        channels: render_status_channels(channels),
        access_reason,
    });
    Ok(Html(page(&username, &body)))
}
