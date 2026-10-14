use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/token/info",
    responses(
        (status = 200, body = Vec<TokenInfoResponse>),
        (status = 401, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub(super) async fn token_info(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
) -> ApiResult<Vec<TokenInfoResponse>> {
    let tokens = sqlx::query_as::<_, TokenInfoResponse>(
        "SELECT id,
                FLOOR(EXTRACT(EPOCH FROM created_at))::bigint AS created_at,
                FLOOR(EXTRACT(EPOCH FROM valid_from))::bigint AS valid_from,
                FLOOR(EXTRACT(EPOCH FROM valid_to))::bigint AS valid_to,
                FLOOR(EXTRACT(EPOCH FROM last_used_at))::bigint AS last_used_at
         FROM api_tokens
         WHERE discord_id = $1
         ORDER BY created_at, id",
    )
    .bind(user.discord_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database)?;
    Ok(Json(tokens))
}
