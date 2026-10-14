use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/metadata",
    params(MetadataQuery),
    responses(
        (status = 200, body = i64),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub(super) async fn metadata_lookup(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Query(query): Query<MetadataQuery>,
) -> ApiResult<i64> {
    if !query.ltype.supports(query.mtype) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "metadata lookup is not supported for this type",
        ));
    }
    let entity_filter = match query.mtype {
        MetadataType::Message => "m.message_id = $1",
        MetadataType::Server => "m.guild_id = $1",
        MetadataType::Channel => "m.channel_id = $1",
        MetadataType::User => "m.author_id = $1",
    };
    let aggregate = match query.ltype {
        MetadataLookup::MessageCount => "COUNT(*)",
        MetadataLookup::FirstMessage => {
            "(ARRAY_AGG(message_id ORDER BY timestamp ASC, message_id ASC))[1]"
        }
        MetadataLookup::LastMessage => {
            "(ARRAY_AGG(message_id ORDER BY timestamp DESC, message_id DESC))[1]"
        }
        MetadataLookup::AttachmentCount => {
            "(SELECT COUNT(*) FROM attachments a WHERE a.message_id = visible.message_id)"
        }
        MetadataLookup::EmbedCount => {
            "(SELECT COUNT(*) FROM embeds e WHERE e.message_id = visible.message_id)"
        }
        MetadataLookup::UserCount => "COUNT(DISTINCT author_id)",
        MetadataLookup::ChannelCount => "COUNT(DISTINCT channel_id)",
    };

    let statement = if matches!(
        query.ltype,
        MetadataLookup::AttachmentCount | MetadataLookup::EmbedCount
    ) {
        format!(
            "SELECT COALESCE(SUM({aggregate}), 0)::bigint FROM (
                 SELECT m.message_id, m.author_id, m.channel_id, m.timestamp
                 FROM messages m
                 WHERE {entity_filter}
                   AND (m.channel_id = ANY($2) OR m.author_id = $3)
             ) visible"
        )
    } else {
        format!(
            "SELECT {aggregate} FROM (
                 SELECT m.message_id, m.author_id, m.channel_id, m.timestamp
                 FROM messages m
                 WHERE {entity_filter}
                   AND (m.channel_id = ANY($2) OR m.author_id = $3)
             ) visible"
        )
    };

    let value = sqlx::query_scalar::<_, Option<i64>>(sqlx::AssertSqlSafe(statement))
        .bind(query.id)
        .bind(&user.channel_ids)
        .bind(user.discord_id)
        .fetch_one(&data.pool)
        .await
        .map_err(database)?
        .ok_or_else(not_found)?;

    Ok(Json(value))
}
