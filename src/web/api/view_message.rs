use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/view/message/{id}",
    params(("id" = i64, Path), VersionQuery),
    responses(
        (status = 200, body = MessageResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub(super) async fn view_message(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
    Query(query): Query<VersionQuery>,
) -> ApiResult<MessageResponse> {
    let message = sqlx::query_as::<_, (i64, i64, String, i64, String, i64, String, String, bool)>(
        "SELECT m.message_id, m.author_id, m.author_username, m.guild_id, g.guild_name,
                m.channel_id, c.channel_name, m.timestamp::text, m.archive_incomplete FROM messages m
         JOIN guilds g ON g.guild_id = m.guild_id JOIN channels c ON c.channel_id = m.channel_id
         WHERE m.message_id = $1 AND (m.channel_id = ANY($2) OR m.author_id = $3)",
    )
    .bind(id)
    .bind(&user.channel_ids)
    .bind(user.discord_id)
    .fetch_optional(&data.pool)
    .await
    .map_err(database)?
    .ok_or_else(not_found)?;
    let version = sqlx::query_as::<_, (i64, Option<String>, String)>(
        "SELECT version, content, archived_at::text FROM message_versions
         WHERE message_id = $1 AND ($2::bigint IS NULL OR version = $2) ORDER BY version DESC LIMIT 1")
        .bind(id).bind(query.version).fetch_optional(&data.pool).await.map_err(database)?.ok_or_else(not_found)?;
    let attachments = sqlx::query_as::<_, AttachmentResponse>(
        "SELECT attachment_id AS discord_id, filename, description, content_type, size FROM attachments
         WHERE message_id = $1 AND message_version = $2 ORDER BY attachment_id")
        .bind(id).bind(version.0).fetch_all(&data.pool).await.map_err(database)?;
    let embeds = sqlx::query_as::<_, EmbedResponse>(
        "SELECT embed_index AS index, title, description, url FROM embeds
         WHERE message_id = $1 AND message_version = $2 ORDER BY embed_index",
    )
    .bind(id)
    .bind(version.0)
    .fetch_all(&data.pool)
    .await
    .map_err(database)?;
    Ok(Json(MessageResponse {
        discord_id: message.0,
        author_id: message.1,
        author_username: message.2,
        server_id: message.3,
        server_name: message.4,
        channel_id: message.5,
        channel_name: message.6,
        timestamp: message.7,
        archive_incomplete: message.8,
        version: version.0,
        content: version.1,
        archived_at: version.2,
        attachments,
        embeds,
    }))
}
