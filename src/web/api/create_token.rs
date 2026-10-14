use super::*;

#[utoipa::path(
    post,
    path = "/api/v1/token",
    params(CreateTokenQuery),
    responses(
        (status = 200, body = TokenResponse),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 429, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    )
)]
pub(super) async fn create_token(
    State(data): State<WebData>,
    session: Session,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<CreateTokenQuery>,
) -> Response {
    let user = match session.get::<WebUser>("user").await {
        Ok(Some(user)) => user,
        Ok(None) => return api_error(StatusCode::UNAUTHORIZED, "browser login required"),
        Err(_) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "session error"),
    };
    if query
        .valid_from
        .zip(query.valid_to)
        .is_some_and(|(valid_from, valid_to)| valid_from > valid_to)
    {
        return api_error(
            StatusCode::BAD_REQUEST,
            "valid_from must not exceed valid_to",
        );
    }
    if !valid_unix_timestamp(query.valid_from) || !valid_unix_timestamp(query.valid_to) {
        return api_error(StatusCode::BAD_REQUEST, "invalid Unix timestamp");
    }
    let client_ip = data.token_rate_limiter.client_ip(peer.ip(), &headers);
    match data.token_rate_limiter.check(client_ip).await {
        Ok(Ok(())) => {}
        Ok(Err(retry_after)) => return rate_limited(retry_after),
        Err(error) => {
            tracing::error!("Token rate limit check failed: {}", error);
            return api_error(StatusCode::SERVICE_UNAVAILABLE, "rate limiter unavailable");
        }
    }
    let token: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();
    let mut transaction = match data.pool.begin().await {
        Ok(transaction) => transaction,
        Err(error_value) => return database(error_value).into_response(),
    };
    let result = sqlx::query(
        "INSERT INTO discord_users (discord_id, discord_username)
         VALUES ($1, $2)
         ON CONFLICT (discord_id) DO UPDATE
         SET discord_username = EXCLUDED.discord_username",
    )
    .bind(user.id)
    .bind(&user.username)
    .execute(&mut *transaction)
    .await;
    if let Err(error_value) = result {
        return database(error_value).into_response();
    }
    let result = sqlx::query_scalar::<_, i64>(
        "INSERT INTO api_tokens (token_hash, discord_id, valid_from, valid_to)
         VALUES ($1, $2, TO_TIMESTAMP($3), TO_TIMESTAMP($4))
         RETURNING id",
    )
    .bind(token_hash(&token))
    .bind(user.id)
    .bind(query.valid_from.map(|value| value as f64))
    .bind(query.valid_to.map(|value| value as f64))
    .fetch_one(&mut *transaction)
    .await;
    let token_id = match result {
        Ok(token_id) => token_id,
        Err(error_value) => return database(error_value).into_response(),
    };
    if let Err(error_value) = transaction.commit().await {
        return database(error_value).into_response();
    }
    Json(TokenResponse {
        id: token_id,
        token,
    })
    .into_response()
}
