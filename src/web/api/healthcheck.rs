use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/healthcheck",
    responses(
        (status = 200, content_type = "text/plain", body = String),
        (status = 503, content_type = "text/plain", body = String)
    )
)]
pub(super) async fn healthcheck(State(data): State<WebData>) -> impl IntoResponse {
    let database = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&data.pool)
        .await;
    let redis = data.redis.ping::<String>(None).await;
    if database.is_ok() && redis.is_ok() {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "unhealthy")
    }
}
