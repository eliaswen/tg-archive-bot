use super::*;

pub(super) async fn semantic_query(data: &WebData, search_by: &str, search: &str) -> Result<Option<Vec<f32>>, (StatusCode, Html<String>)> {
    if search_by != "image" {
        return Ok(None);
    }
    if search.trim().is_empty() {
        return Err(bad_request("Enter a search query for image search."));
    }
    let Some(encoder) = data.text_encoder.clone() else {
        return Err((StatusCode::SERVICE_UNAVAILABLE, Html(page("Image search unavailable", "Image search is temporarily unavailable."))));
    };
    let _permit = data.ml_semaphore.clone().acquire_owned().await.map_err(|_| {
        (StatusCode::SERVICE_UNAVAILABLE, Html(page("Image search unavailable", "Image search is temporarily unavailable.")))
    })?;
    let search = search.to_owned();
    tokio::task::spawn_blocking(move || encoder.embed(&search))
        .await
        .map_err(|error| database_error(sqlx::Error::Protocol(error.to_string())))?
        .map(Some)
        .map_err(|error| {
            error!("Could not encode image search query: {error}");
            (StatusCode::SERVICE_UNAVAILABLE, Html(page("Image search unavailable", "Image search is temporarily unavailable.")))
        })
}

pub(in crate::web) async fn messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_messages(
        &data.pool,
        "",
        "content",
        None,
        query.page.unwrap_or(1),
        MessageScope::All,
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(in crate::web) async fn search_messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Form(form): Form<MessageSearchForm>,
) -> WebResult {
    if !csrf_token_is_valid(&form.csrf_token) {
        return Err(csrf_error());
    }
    let embedding = semantic_query(&data, &form.search_by, &form.search).await?;
    render_messages(
        &data.pool,
        &form.search,
        &form.search_by,
        embedding,
        form.page.unwrap_or(1),
        MessageScope::All,
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(in crate::web) async fn render_messages(
    pool: &PgPool,
    search: &str,
    search_by: &str,
    embedding: Option<Vec<f32>>,
    requested_page: i64,
    scope: MessageScope,
    channel_ids: &[i64],
    viewer_id: i64,
) -> WebResult {
    let search_by = match search_by {
        "timestamp" => "timestamp",
        "image" => "image",
        _ => "content",
    };
    if search_by == "image" {
        return render_image_messages(pool, search, embedding, requested_page, scope, channel_ids, viewer_id).await;
    }
    let search_pattern = format!("%{}%", search);
    let item_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM messages m
         JOIN guilds g ON g.guild_id = m.guild_id
         JOIN channels c ON c.channel_id = m.channel_id
         WHERE CASE
                   WHEN $2 = 'timestamp' THEN m.timestamp::text ILIKE $1
                   ELSE COALESCE(m.content, '') ILIKE $1
               END
           AND ($3::bigint IS NULL OR m.guild_id = $3)
           AND ($4::bigint IS NULL OR m.author_id = $4)
           AND ($5::bigint IS NULL OR m.channel_id = $5)
           AND (m.channel_id = ANY($6) OR m.author_id = $7);",
    )
    .bind(&search_pattern)
    .bind(search_by)
    .bind(scope.server_id())
    .bind(scope.user_id())
    .bind(scope.channel_id())
    .bind(channel_ids)
    .bind(viewer_id)
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    let pagination = Pagination::new(requested_page, item_count);
    let messages =
        sqlx::query_as::<_, (i64, i64, String, i64, String, i64, String, String, i64, i64)>(
            "SELECT
             m.message_id,
             m.author_id,
             m.author_username,
             m.guild_id,
             g.guild_name,
             m.channel_id,
             c.channel_name,
             m.timestamp::text,
             (SELECT COUNT(*)
              FROM attachments a
              WHERE a.message_id = m.message_id
                AND a.message_version = (
                    SELECT MAX(version) FROM message_versions WHERE message_id = m.message_id
                )),
             (SELECT COUNT(*)
              FROM embeds e
              WHERE e.message_id = m.message_id
                AND e.message_version = (
                    SELECT MAX(version) FROM message_versions WHERE message_id = m.message_id
                ))
         FROM messages m
         JOIN guilds g ON g.guild_id = m.guild_id
         JOIN channels c ON c.channel_id = m.channel_id
         WHERE CASE
                   WHEN $2 = 'timestamp' THEN m.timestamp::text ILIKE $1
                   ELSE COALESCE(m.content, '') ILIKE $1
               END
           AND ($3::bigint IS NULL OR m.guild_id = $3)
           AND ($4::bigint IS NULL OR m.author_id = $4)
           AND ($5::bigint IS NULL OR m.channel_id = $5)
           AND (m.channel_id = ANY($6) OR m.author_id = $7)
         ORDER BY m.timestamp DESC, m.message_id DESC
         LIMIT $8 OFFSET $9;",
        )
        .bind(&search_pattern)
        .bind(search_by)
        .bind(scope.server_id())
        .bind(scope.user_id())
        .bind(scope.channel_id())
        .bind(channel_ids)
        .bind(viewer_id)
        .bind(ITEMS_PER_PAGE)
        .bind(pagination.offset())
        .fetch_all(pool)
        .await
        .map_err(database_error)?;

    let mut items = String::new();
    for (
        message_id,
        author_id,
        author,
        server_id,
        server,
        channel_id,
        channel,
        timestamp,
        attachment_count,
        embed_count,
    ) in messages
    {
        let timestamp = localize_timestamp(&timestamp);
        items.push_str(&render_template(&MessageListItemTemplate {
            message_id,
            message_version: None,
            attachment_filename: None,
            author_id,
            author: &author,
            server_id,
            server: &server,
            channel_id,
            channel: &channel,
            timestamp: &timestamp,
            attachment_count,
            embed_count,
        }));
    }

    let title = scope.title();
    let action = scope.action();
    let body = render_template(&MessageListTemplate {
        title: &title,
        search_form: message_search_form(&action, search, search_by),
        item_count,
        items,
        pagination: pagination.render_messages(&action, search, search_by),
    });
    Ok(Html(page(&title, &body)))
}

async fn render_image_messages(
    pool: &PgPool,
    _search: &str,
    embedding: Option<Vec<f32>>,
    requested_page: i64,
    scope: MessageScope,
    channel_ids: &[i64],
    viewer_id: i64,
) -> WebResult {
    let Some(embedding) = embedding else {
        return Err((StatusCode::SERVICE_UNAVAILABLE, Html(page("Image search unavailable", "Image search is temporarily unavailable."))));
    };
    let embedding = pgvector::Vector::from(embedding);
    let scope_server_id = scope.server_id();
    let scope_user_id = scope.user_id();
    let scope_channel_id = scope.channel_id();
    let item_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM image_embeddings ie
         JOIN messages m ON m.message_id = ie.message_id
         WHERE ($1::bigint IS NULL OR m.guild_id = $1)
           AND ($2::bigint IS NULL OR m.author_id = $2)
           AND ($3::bigint IS NULL OR m.channel_id = $3)
           AND (m.channel_id = ANY($4) OR m.author_id = $5);",
    )
    .bind(scope_server_id)
    .bind(scope_user_id)
    .bind(scope_channel_id)
    .bind(channel_ids)
    .bind(viewer_id)
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    let pagination = Pagination::new(requested_page, item_count);
    let results = sqlx::query_as::<_, (i64, i64, String, i64, String, i64, String, String, i64, i64, i64, String)>(
        "SELECT m.message_id, m.author_id, m.author_username, m.guild_id, g.guild_name,
                m.channel_id, c.channel_name, m.timestamp::text, ie.message_version,
                (SELECT COUNT(*) FROM attachments a WHERE a.message_id = m.message_id AND a.message_version = ie.message_version),
                (SELECT COUNT(*) FROM embeds e WHERE e.message_id = m.message_id AND e.message_version = ie.message_version),
                a.filename
         FROM image_embeddings ie
         JOIN messages m ON m.message_id = ie.message_id
         JOIN guilds g ON g.guild_id = m.guild_id
         JOIN channels c ON c.channel_id = m.channel_id
         JOIN attachments a ON a.message_id = ie.message_id AND a.message_version = ie.message_version AND a.attachment_id = ie.attachment_id
         WHERE ($2::bigint IS NULL OR m.guild_id = $2)
           AND ($3::bigint IS NULL OR m.author_id = $3)
           AND ($4::bigint IS NULL OR m.channel_id = $4)
           AND (m.channel_id = ANY($5) OR m.author_id = $6)
         ORDER BY ie.embedding <=> $1, ie.message_id, ie.message_version, ie.attachment_id
         LIMIT $7 OFFSET $8;",
    )
    .bind(&embedding)
    .bind(scope_server_id)
    .bind(scope_user_id)
    .bind(scope_channel_id)
    .bind(channel_ids)
    .bind(viewer_id)
    .bind(ITEMS_PER_PAGE)
    .bind(pagination.offset())
    .fetch_all(pool)
    .await
    .map_err(database_error)?;
    let mut items = String::new();
    for (message_id, author_id, author, server_id, server, channel_id, channel, timestamp, version, attachment_count, embed_count, filename) in results {
        let timestamp = localize_timestamp(&timestamp);
        items.push_str(&render_template(&MessageListItemTemplate {
            message_id,
            message_version: Some(version),
            attachment_filename: Some(&filename),
            author_id,
            author: &author,
            server_id,
            server: &server,
            channel_id,
            channel: &channel,
            timestamp: &timestamp,
            attachment_count,
            embed_count,
        }));
    }
    let title = scope.title();
    let action = scope.action();
    let body = render_template(&MessageListTemplate {
        title: &title,
        search_form: message_search_form(&action, _search, "image"),
        item_count,
        items,
        pagination: pagination.render_messages(&action, _search, "image"),
    });
    Ok(Html(page(&title, &body)))
}
