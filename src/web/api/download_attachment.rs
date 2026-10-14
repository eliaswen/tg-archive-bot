use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/download/attachment/{id}",
    params(("id" = i64, Path), VersionQuery),
    responses(
        (status = 200, content_type = "application/octet-stream", body = Vec<u8>),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub(super) async fn download_attachment(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
    Query(query): Query<VersionQuery>,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let row = sqlx::query_as::<_, (String, Option<String>, Vec<u8>)>(
        "SELECT a.filename, a.content_type, a.data FROM attachments a
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

    let content_type = row
        .1
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", safe_filename(&row.0)),
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        row.2,
    )
        .into_response())
}
