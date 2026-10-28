use super::*;

pub(in crate::web) async fn attachment(
    State(data): State<WebData>,
    Path(attachment_id): Path<i64>,
    Query(query): Query<AttachmentQuery>,
) -> Response {
    let attachment = sqlx::query_as::<_, (String, Option<String>, Vec<u8>)>(
        "SELECT filename, content_type, data
         FROM attachments
         WHERE attachment_id = $1 AND ($2::bigint IS NULL OR message_version = $2)
         ORDER BY message_version DESC
         LIMIT 1;",
    )
    .bind(attachment_id)
    .bind(query.version)
    .fetch_optional(&data.pool)
    .await;

    match attachment {
        Ok(Some((filename, content_type, data))) => {
            let content_type = content_type.unwrap_or_else(|| "application/octet-stream".into());
            let disposition = if is_inline_content_type(&content_type) {
                "inline"
            } else {
                "attachment"
            };
            (
                [
                    (header::CONTENT_TYPE, content_type),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("{}; filename=\"{}\"", disposition, safe_filename(&filename)),
                    ),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
                    (
                        header::CONTENT_SECURITY_POLICY,
                        "sandbox; default-src 'none'; img-src 'self' data:; media-src 'self'"
                            .into(),
                    ),
                ],
                data,
            )
                .into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            error!("Database error while loading attachment: {}", error);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
