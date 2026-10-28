use super::*;

pub(in crate::web) async fn avatar(
    State(data): State<WebData>,
    Path(avatar_id): Path<String>,
) -> Response {
    let Ok(avatar_id) = sqlx::types::Uuid::parse_str(&avatar_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let avatar = sqlx::query_as::<_, (Vec<u8>, Option<String>)>(
        "SELECT image_data, image_mimetype
         FROM discord_user_history
         WHERE uuid = $1 AND image_checked = true AND image_available = true;",
    )
    .bind(avatar_id)
    .fetch_optional(&data.pool)
    .await;

    match avatar {
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
            error!("Database error while loading avatar: {}", error);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
