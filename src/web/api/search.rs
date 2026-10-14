use super::*;

pub(super) async fn search(
    data: &WebData,
    user: &ApiUser,
    query: &SearchQuery,
    limit: &i64,
    timestamp: bool,
) -> ApiResult<SearchResponse> {
    let limit = limit.max(&1);
    let pattern = format!("%{}%", query.q);
    let fields = parse_search_filter(query.filter.as_deref())?;

    let results = sqlx::query_as::<_, MessageSummary>(
        "SELECT m.message_id AS discord_id, m.author_id, m.author_username, m.guild_id AS server_id,
         g.guild_name AS server_name, m.channel_id, c.channel_name, m.timestamp::text AS timestamp, m.content
         FROM messages m
         JOIN guilds g ON g.guild_id = m.guild_id
         JOIN channels c ON c.channel_id = m.channel_id
         WHERE CASE
             WHEN $2 THEN m.timestamp::text ILIKE $1
             ELSE COALESCE(m.content, '') ILIKE $1
         END
         AND (m.channel_id = ANY($3) OR m.author_id = $4)
         AND ($6::bigint[] IS NULL OR m.guild_id = ANY($6))
         AND ($7::bigint[] IS NULL OR m.channel_id = ANY($7))
         ORDER BY m.timestamp DESC, m.message_id DESC
         LIMIT $5"
    )
    .bind(&pattern)
    .bind(timestamp)
    .bind(&user.channel_ids)
    .bind(user.discord_id)
    .bind(limit)
    .bind(query.server_scope.as_deref())
    .bind(query.channel_scope.as_deref())
    .fetch_all(&data.pool)
    .await
    .map_err(database)?;

    let results = results
        .into_iter()
        .map(|result| FilteredMessageSummary {
            discord_id: fields[0].then_some(result.discord_id),
            author_id: fields[1].then_some(result.author_id),
            author_username: fields[2].then_some(result.author_username),
            server_id: fields[3].then_some(result.server_id),
            server_name: fields[4].then_some(result.server_name),
            channel_id: fields[5].then_some(result.channel_id),
            channel_name: fields[6].then_some(result.channel_name),
            timestamp: fields[7].then_some(result.timestamp),
            content: fields[8].then_some(result.content),
        })
        .collect();

    Ok(Json(SearchResponse {
        query: query.q.clone(),
        limit: *limit,
        results,
    }))
}

fn parse_search_filter(
    filter: Option<&str>,
) -> Result<[bool; 9], (StatusCode, Json<ErrorResponse>)> {
    let Some(filter) = filter else {
        return Ok([true; 9]);
    };
    let mut selected = [false; 9];
    for field in filter.split(',').map(str::trim) {
        let Some(index) = SEARCH_FIELDS
            .iter()
            .position(|candidate| *candidate == field)
        else {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid search filter field",
            ));
        };
        selected[index] = true;
    }
    Ok(selected)
}