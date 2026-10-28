use super::*;

pub(in crate::web) async fn icon(
    State(data): State<WebData>,
    Path(icon_id): Path<String>,
) -> Response {
    let Ok(icon_id) = sqlx::types::Uuid::parse_str(&icon_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let icon = sqlx::query_as::<_, (Vec<u8>, Option<String>)>(
        "SELECT image_data, image_mimetype
         FROM guild_history
         WHERE uuid = $1 AND image_checked = true AND image_available = true;",
    )
    .bind(icon_id)
    .fetch_optional(&data.pool)
    .await;

    match icon {
        Ok(Some((image_data, image_mimetype))) => {
            let content_type = image_mimetype.unwrap_or_else(|| "application/octet-stream".into());
            (
                [
                    (header::CONTENT_TYPE, content_type),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
                ],
                image_data,
            )
                .into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            error!("Database error while loading icon: {}", error);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
