use super::*;

pub(in crate::web) async fn server_messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(server_id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_server_messages(
        &data.pool,
        server_id,
        "",
        "content",
        None,
        query.page.unwrap_or(1),
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(in crate::web) async fn search_server_messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(server_id): Path<i64>,
    Form(form): Form<MessageSearchForm>,
) -> WebResult {
    if !csrf_token_is_valid(&form.csrf_token) {
        return Err(csrf_error());
    }
    let embedding = semantic_query(&data, &form.search_by, &form.search).await?;
    render_server_messages(
        &data.pool,
        server_id,
        &form.search,
        &form.search_by,
        embedding,
        form.page.unwrap_or(1),
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(in crate::web) async fn render_server_messages(
    pool: &PgPool,
    server_id: i64,
    search: &str,
    search_by: &str,
    embedding: Option<Vec<f32>>,
    requested_page: i64,
    channel_ids: &[i64],
    viewer_id: i64,
) -> WebResult {
    let server_name =
        sqlx::query_scalar::<_, String>("SELECT guild_name FROM guilds WHERE guild_id = $1;")
            .bind(server_id)
            .fetch_optional(pool)
            .await
            .map_err(database_error)?
            .ok_or_else(|| not_found("Server not found."))?;
    render_messages(
        pool,
        search,
        search_by,
        embedding,
        requested_page,
        MessageScope::Server(server_id, server_name),
        channel_ids,
        viewer_id,
    )
    .await
}
