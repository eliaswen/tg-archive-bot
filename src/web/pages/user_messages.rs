use super::*;

pub(in crate::web) async fn user_messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(user_id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_user_messages(
        &data.pool,
        user_id,
        "",
        "content",
        None,
        query.page.unwrap_or(1),
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(in crate::web) async fn search_user_messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(user_id): Path<i64>,
    Form(form): Form<MessageSearchForm>,
) -> WebResult {
    if !csrf_token_is_valid(&form.csrf_token) {
        return Err(csrf_error());
    }
    let embedding = semantic_query(&data, &form.search_by, &form.search).await?;
    render_user_messages(
        &data.pool,
        user_id,
        &form.search,
        &form.search_by,
        embedding,
        form.page.unwrap_or(1),
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(in crate::web) async fn render_user_messages(
    pool: &PgPool,
    user_id: i64,
    search: &str,
    search_by: &str,
    embedding: Option<Vec<f32>>,
    requested_page: i64,
    channel_ids: &[i64],
    viewer_id: i64,
) -> WebResult {
    let username = sqlx::query_scalar::<_, String>(
        "SELECT discord_username FROM discord_users WHERE discord_id = $1;",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(database_error)?
    .ok_or_else(|| not_found("User not found."))?;
    render_messages(
        pool,
        search,
        search_by,
        embedding,
        requested_page,
        MessageScope::User(user_id, username),
        channel_ids,
        viewer_id,
    )
    .await
}
