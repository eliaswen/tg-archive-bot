use super::*;

pub(in crate::web) async fn servers(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_servers(&data.pool, "", query.page.unwrap_or(1), &user.channel_ids).await
}

pub(in crate::web) async fn search_servers(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Form(form): Form<SearchForm>,
) -> WebResult {
    if !csrf_token_is_valid(&form.csrf_token) {
        return Err(csrf_error());
    }
    render_servers(
        &data.pool,
        &form.search,
        form.page.unwrap_or(1),
        &user.channel_ids,
    )
    .await
}

pub(in crate::web) async fn render_servers(
    pool: &PgPool,
    search: &str,
    requested_page: i64,
    channel_ids: &[i64],
) -> WebResult {
    let search_pattern = format!("%{}%", search);
    let item_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM guilds
         WHERE (guild_name ILIKE $1 OR guild_id::text ILIKE $1)
           AND EXISTS(SELECT 1 FROM channels c WHERE c.guild_id = guilds.guild_id AND c.channel_id = ANY($2));",
    )
    .bind(&search_pattern)
    .bind(channel_ids)
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    let pagination = Pagination::new(requested_page, item_count);
    let servers = sqlx::query_as::<_, (i64, String, Option<String>)>(
        "SELECT guild_id, guild_name, COALESCE(
             (SELECT '/icons/' || h.uuid::text FROM guild_history h
              WHERE h.guild_id = guilds.guild_id AND h.guild_icon_url = guilds.guild_icon_url
                AND h.image_available = true ORDER BY h.last_seen_at DESC LIMIT 1), guild_icon_url)
         FROM guilds
         WHERE (guild_name ILIKE $1 OR guild_id::text ILIKE $1)
           AND EXISTS(SELECT 1 FROM channels c WHERE c.guild_id = guilds.guild_id AND c.channel_id = ANY($2))
         ORDER BY guild_name, guild_id
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
    for (guild_id, guild_name, guild_icon_url) in servers {
        let icon = guild_icon_url
            .map(|icon_url| {
                render_template(&ServerIconTemplate {
                    icon_url: &icon_url,
                })
            })
            .unwrap_or_default();
        items.push_str(&render_template(&ServerTemplate {
            guild_id,
            guild_name: &guild_name,
            icon,
        }));
    }

    let body = render_template(&ListTemplate {
        title: "Servers",
        search_form: search_form("/servers", search, "Search servers"),
        item_count,
        item_name: "archived servers",
        items,
        pagination: pagination.render("/servers", search),
    });
    Ok(Html(page("Servers", &body)))
}
