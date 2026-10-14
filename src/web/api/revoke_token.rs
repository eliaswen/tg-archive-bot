use super::*;

#[utoipa::path(
    post,
    path = "/api/v1/token/revoke",
    responses(
        (status = 204),
        (status = 401, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub(super) async fn revoke_token(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
) -> Response {
    match sqlx::query("DELETE FROM api_tokens WHERE id = $1 AND discord_id = $2")
        .bind(user.token_id)
        .bind(user.discord_id)
        .execute(&data.pool)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error_value) => database(error_value).into_response(),
    }
}

pub(super) async fn invalid_method_use_post() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, HeaderValue::from_static("POST"))], Json(ErrorResponse { error: "invalid HTTP method, use POST instead" }), ).into_response()
}
