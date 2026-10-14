use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/me",
    responses((status = 200, body = MeResponse), (status = 401, body = ErrorResponse)),
    security(("bearer_auth" = []))
)]
pub(super) async fn me(Extension(user): Extension<ApiUser>) -> Json<MeResponse> {
    Json(MeResponse {
        discord_id: user.discord_id,
    })
}
