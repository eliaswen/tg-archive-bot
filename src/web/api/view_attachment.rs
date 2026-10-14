use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/view/attachment/{id}",
    params(("id" = i64, Path), VersionQuery),
    responses(
        (status = 200, body = AttachmentResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub(super) async fn view_attachment(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
    Query(query): Query<VersionQuery>,
) -> ApiResult<AttachmentResponse> {
    let row = sqlx::query_as::<_, AttachmentResponse>(
        "SELECT attachment_id AS discord_id, filename, description, content_type, size FROM attachments a
         JOIN messages m ON m.message_id = a.message_id
         WHERE a.attachment_id = $1 AND (m.channel_id = ANY($2) OR m.author_id = $3)
           AND ($4::bigint IS NULL OR a.message_version = $4)
         ORDER BY a.message_version DESC
         LIMIT 1",
    )
    .bind(id)
    .bind(&user.channel_ids)
    .bind(user.discord_id)
    .bind(query.version)
    .fetch_optional(&data.pool)
    .await
    .map_err(database)?
    .ok_or_else(not_found)?;
    Ok(Json(row))
}
