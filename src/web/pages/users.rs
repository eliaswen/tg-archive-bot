use super::*;

pub(in crate::web) async fn users(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_users(&data.pool, "", query.page.unwrap_or(1), &user.channel_ids).await
}

pub(in crate::web) async fn search_users(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Form(form): Form<SearchForm>,
) -> WebResult {
    if !csrf_token_is_valid(&form.csrf_token) {
        return Err(csrf_error());
    }
    render_users(
        &data.pool,
        &form.search,
        form.page.unwrap_or(1),
        &user.channel_ids,
    )
    .await
}

pub(in crate::web) async fn render_users(
    pool: &PgPool,
    search: &str,
    requested_page: i64,
    channel_ids: &[i64],
) -> WebResult {
    let search_pattern = format!("%{}%", search);
    let item_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM discord_users
         WHERE (discord_username ILIKE $1 OR discord_id::text ILIKE $1)
           AND EXISTS(SELECT 1 FROM messages m WHERE m.author_id = discord_users.discord_id AND m.channel_id = ANY($2));",
    )
    .bind(&search_pattern)
    .bind(channel_ids)
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    let pagination = Pagination::new(requested_page, item_count);
    let users = sqlx::query_as::<_, (i64, String)>(
        "SELECT discord_id, discord_username
         FROM discord_users
         WHERE (discord_username ILIKE $1 OR discord_id::text ILIKE $1)
           AND EXISTS(SELECT 1 FROM messages m WHERE m.author_id = discord_users.discord_id AND m.channel_id = ANY($2))
         ORDER BY discord_username, discord_id
         LIMIT $3 OFFSET $4;",
    )
    .bind(&search_pattern)
    .bind(channel_ids)
    .bind(ITEMS_PER_PAGE)
    .bind(pagination.offset())
    .fetch_all(pool)
    .await
    .map_err(database_error)?;

    let mut items = String::new();
    for (discord_id, discord_username) in users {
        items.push_str(&render_template(&UserTemplate {
            discord_id,
            discord_username: &discord_username,
        }));
    }

    let body = render_template(&ListTemplate {
        title: "Users",
        search_form: search_form("/users", search, "Search users"),
        item_count,
        item_name: "archived users",
        items,
        pagination: pagination.render("/users", search),
    });
    Ok(Html(page("Users", &body)))
}
