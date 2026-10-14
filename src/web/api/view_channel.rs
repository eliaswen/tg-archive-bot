use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/view/channel/{id}",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = ChannelResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub(super) async fn view_channel(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
) -> ApiResult<ChannelResponse> {
    let row = sqlx::query_as::<_, (i64, String, i64, String)>(
        "SELECT c.channel_id, c.channel_name, g.guild_id, g.guild_name FROM channels c
         JOIN guilds g ON g.guild_id = c.guild_id WHERE c.channel_id = $1 AND c.channel_id = ANY($2)")
        .bind(id).bind(&user.channel_ids).fetch_optional(&data.pool).await.map_err(database)?.ok_or_else(not_found)?;
    Ok(Json(ChannelResponse {
        discord_id: row.0,
        name: row.1,
        server_id: row.2,
        server_name: row.3,
    }))
}
