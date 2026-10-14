use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/view/user/{id}",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = UserResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub(super) async fn view_user(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
) -> ApiResult<UserResponse> {
    let row = sqlx::query_as::<_, (i64, String)>(
        "SELECT u.discord_id, u.discord_username FROM discord_users u WHERE u.discord_id = $1
         AND (u.discord_id = $2 OR EXISTS (SELECT 1 FROM messages m
              WHERE m.author_id = u.discord_id AND m.channel_id = ANY($3)))",
    )
    .bind(id)
    .bind(user.discord_id)
    .bind(&user.channel_ids)
    .fetch_optional(&data.pool)
    .await
    .map_err(database)?
    .ok_or_else(not_found)?;
    let history = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
        "SELECT
             (ARRAY_AGG(discord_avatar_url ORDER BY last_seen_at DESC)
                 FILTER (WHERE discord_avatar_url IS NOT NULL))[1],
             MIN(first_seen_at)::text,
             MAX(last_seen_at)::text
         FROM discord_user_history WHERE discord_id = $1",
    )
    .bind(id)
    .fetch_one(&data.pool)
    .await
    .map_err(database)?;
    let counts = sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(
        "SELECT COUNT(DISTINCT m.message_id), COUNT(DISTINCT m.guild_id),
                COUNT(DISTINCT m.channel_id), COUNT(DISTINCT a.uuid), COUNT(DISTINCT e.uuid)
         FROM messages m
         LEFT JOIN attachments a ON a.message_id = m.message_id
         LEFT JOIN embeds e ON e.message_id = m.message_id
         WHERE m.author_id = $1 AND (m.channel_id = ANY($2) OR m.author_id = $3)",
    )
    .bind(id)
    .bind(&user.channel_ids)
    .bind(user.discord_id)
    .fetch_one(&data.pool)
    .await
    .map_err(database)?;
    let usernames = sqlx::query_as::<_, UserNameResponse>(
        "SELECT discord_username AS username, MIN(first_seen_at)::text AS first_seen_at,
                MAX(last_seen_at)::text AS last_seen_at
         FROM discord_user_history WHERE discord_id = $1
         GROUP BY discord_username ORDER BY MIN(first_seen_at), discord_username",
    )
    .bind(id)
    .fetch_all(&data.pool)
    .await
    .map_err(database)?;
    let avatars = sqlx::query_as::<_, UserAvatarResponse>(
        "SELECT discord_avatar_url AS url, MIN(first_seen_at)::text AS first_seen_at,
                MAX(last_seen_at)::text AS last_seen_at
         FROM discord_user_history
         WHERE discord_id = $1 AND discord_avatar_url IS NOT NULL
         GROUP BY discord_avatar_url ORDER BY MIN(first_seen_at), discord_avatar_url",
    )
    .bind(id)
    .fetch_all(&data.pool)
    .await
    .map_err(database)?;
    Ok(Json(UserResponse {
        discord_id: row.0,
        username: row.1,
        avatar_url: history.0,
        first_seen_at: history.1,
        last_seen_at: history.2,
        message_count: counts.0,
        server_count: counts.1,
        channel_count: counts.2,
        attachment_count: counts.3,
        embed_count: counts.4,
        usernames,
        avatars,
    }))
}
