use super::*;

pub(in crate::web) async fn channels(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_channels(&data.pool, "", query.page.unwrap_or(1), &user.channel_ids).await
}

pub(in crate::web) async fn search_channels(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Form(form): Form<SearchForm>,
) -> WebResult {
    if !csrf_token_is_valid(&form.csrf_token) {
        return Err(csrf_error());
    }
    render_channels(
        &data.pool,
        &form.search,
        form.page.unwrap_or(1),
        &user.channel_ids,
    )
    .await
}

pub(in crate::web) async fn render_channels(
    pool: &PgPool,
    search: &str,
    requested_page: i64,
    channel_ids: &[i64],
) -> WebResult {
    let search_pattern = format!("%{}%", search);
    let item_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM channels c
         JOIN guilds g ON g.guild_id = c.guild_id
         WHERE (c.channel_name ILIKE $1
            OR c.channel_id::text ILIKE $1
            OR g.guild_name ILIKE $1)
           AND c.channel_id = ANY($2);",
    )
    .bind(&search_pattern)
    .bind(channel_ids)
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    let pagination = Pagination::new(requested_page, item_count);
    let channels = sqlx::query_as::<_, (i64, String, i64, String)>(
        "SELECT c.channel_id, c.channel_name, g.guild_id, g.guild_name
         FROM channels c
         JOIN guilds g ON g.guild_id = c.guild_id
         WHERE (c.channel_name ILIKE $1
            OR c.channel_id::text ILIKE $1
            OR g.guild_name ILIKE $1)
           AND c.channel_id = ANY($2)
         ORDER BY c.channel_name, c.channel_id
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
    for (channel_id, channel_name, server_id, server_name) in channels {
        items.push_str(&render_template(&ChannelTemplate {
            channel_id,
            channel_name: &channel_name,
            server_id,
            server_name: &server_name,
        }));
    }

    let body = render_template(&ListTemplate {
        title: "Channels",
        search_form: search_form("/channels", search, "Search channels"),
        item_count,
        item_name: "archived channels",
        items,
        pagination: pagination.render("/channels", search),
    });
    Ok(Html(page("Channels", &body)))
}
