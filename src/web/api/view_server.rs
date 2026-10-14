use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/view/server/{id}",
    params(("id" = i64, Path)),
    responses(
        (status = 200, body = ServerResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub(super) async fn view_server(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
) -> ApiResult<ServerResponse> {
    let row = sqlx::query_as::<_, (i64, String, Option<String>)>(
        "SELECT DISTINCT g.guild_id, g.guild_name, g.guild_icon_url FROM guilds g
         JOIN channels c ON c.guild_id = g.guild_id
         WHERE g.guild_id = $1 AND c.channel_id = ANY($2)",
    )
    .bind(id)
    .bind(&user.channel_ids)
    .fetch_optional(&data.pool)
    .await
    .map_err(database)?
    .ok_or_else(not_found)?;
    Ok(Json(ServerResponse {
        discord_id: row.0,
        name: row.1,
        icon_url: row.2,
    }))
}
