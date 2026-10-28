use super::*;

pub(in crate::web) async fn server(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(server_id): Path<i64>,
) -> WebResult {
    let server_name =
        sqlx::query_scalar::<_, String>("SELECT guild_name FROM guilds WHERE guild_id = $1;")
            .bind(server_id)
            .fetch_optional(&data.pool)
            .await
            .map_err(database_error)?
            .ok_or_else(|| not_found("Server not found."))?;
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(
        "SELECT
             (SELECT COUNT(*) FROM messages WHERE guild_id = $1),
             (SELECT COUNT(*) FROM guild_users WHERE guild_id = $1),
             (SELECT COUNT(*) FROM channels WHERE guild_id = $1),
             (SELECT COUNT(*)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.guild_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(v.content)), 0)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.guild_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(a.data)), 0)
              FROM attachments a
              JOIN messages m ON m.message_id = a.message_id
              WHERE m.guild_id = $1);",
    )
    .bind(server_id)
    .fetch_one(&data.pool)
    .await
    .map_err(database_error)?;
    let names = sqlx::query_as::<_, (String, String, String)>(
        "SELECT guild_name, MIN(first_seen_at)::text, MAX(last_seen_at)::text
         FROM guild_history
         WHERE guild_id = $1
         GROUP BY guild_name
         ORDER BY MIN(first_seen_at), guild_name;",
    )
    .bind(server_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let icons = sqlx::query_as::<_, (String, String, String)>(
        "SELECT COALESCE(
             (ARRAY_AGG('/icons/' || uuid::text ORDER BY image_available DESC NULLS LAST, last_seen_at DESC)
                 FILTER (WHERE image_available = true))[1], guild_icon_url),
             MIN(first_seen_at)::text, MAX(last_seen_at)::text
         FROM guild_history
         WHERE guild_id = $1 AND guild_icon_url IS NOT NULL
         GROUP BY guild_icon_url
         ORDER BY MIN(first_seen_at), guild_icon_url;",
    )
    .bind(server_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let channels = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT c.channel_id, c.channel_name, COUNT(m.message_id)
         FROM channels c
         LEFT JOIN messages m ON m.channel_id = c.channel_id
         WHERE c.guild_id = $1 AND c.channel_id = ANY($2)
         GROUP BY c.channel_id, c.channel_name
         ORDER BY c.channel_name, c.channel_id;",
    )
    .bind(server_id)
    .bind(&user.channel_ids)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let users = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT u.discord_id, u.discord_username, COUNT(m.message_id)
         FROM guild_users gu
         JOIN discord_users u ON u.discord_id = gu.discord_id
         JOIN messages m ON m.guild_id = gu.guild_id AND m.author_id = gu.discord_id
         WHERE gu.guild_id = $1 AND m.channel_id = ANY($2)
         GROUP BY u.discord_id, u.discord_username
         ORDER BY u.discord_username, u.discord_id;",
    )
    .bind(server_id)
    .bind(&user.channel_ids)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;

    let body = render_template(&ServerStatusTemplate {
        server_name: &server_name,
        server_id,
        message_count: stats.0,
        user_count: stats.1,
        channel_count: stats.2,
        version_count: stats.3,
        total_storage: format_bytes(stats.4 + stats.5),
        message_storage: format_bytes(stats.4),
        attachment_storage: format_bytes(stats.5),
        names: render_status_names(names),
        icons: render_status_icons(icons),
        channels: render_status_channels(channels),
        users: render_status_users(users),
    });
    Ok(Html(page(&server_name, &body)))
}
