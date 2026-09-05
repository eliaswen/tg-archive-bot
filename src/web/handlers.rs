use super::*;

pub(super) async fn index(State(data): State<WebData>) -> WebResult {
    let stats = crate::archive_stats::load(&data.pool)
        .await
        .map_err(database_error)?;
    let body = render_template(&IndexTemplate {
        message_count: stats.messages,
        user_count: stats.users,
        server_count: stats.servers,
        channel_count: stats.channels,
        total_storage: format_bytes(stats.total_storage),
        message_storage: format_bytes(stats.message_storage),
        attachment_storage: format_bytes(stats.attachment_storage),
    });
    Ok(Html(page("Archive", &body)))
}

pub(super) async fn servers(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_servers(&data.pool, "", query.page.unwrap_or(1), &user.channel_ids).await
}

pub(super) async fn search_servers(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Form(form): Form<SearchForm>,
) -> WebResult {
    render_servers(
        &data.pool,
        &form.search,
        form.page.unwrap_or(1),
        &user.channel_ids,
    )
    .await
}

pub(super) async fn server(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(server_id): Path<i64>,
) -> WebResult {
    let server_name =
        sqlx::query_scalar::<_, String>("SELECT guild_name FROM guilds WHERE guild_id = $1;")
            .bind(server_id)
            .fetch_optional(&data.pool)
            .await
            .map_err(database_error)?
            .ok_or_else(|| not_found("Server not found."))?;
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(
        "SELECT
             (SELECT COUNT(*) FROM messages WHERE guild_id = $1),
             (SELECT COUNT(*) FROM guild_users WHERE guild_id = $1),
             (SELECT COUNT(*) FROM channels WHERE guild_id = $1),
             (SELECT COUNT(*)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.guild_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(v.content)), 0)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.guild_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(a.data)), 0)
              FROM attachments a
              JOIN messages m ON m.message_id = a.message_id
              WHERE m.guild_id = $1);",
    )
    .bind(server_id)
    .fetch_one(&data.pool)
    .await
    .map_err(database_error)?;
    let names = sqlx::query_as::<_, (String, String, String)>(
        "SELECT guild_name, MIN(first_seen_at)::text, MAX(last_seen_at)::text
         FROM guild_history
         WHERE guild_id = $1
         GROUP BY guild_name
         ORDER BY MIN(first_seen_at), guild_name;",
    )
    .bind(server_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let icons = sqlx::query_as::<_, (String, String, String)>(
        "SELECT guild_icon_url, MIN(first_seen_at)::text, MAX(last_seen_at)::text
         FROM guild_history
         WHERE guild_id = $1 AND guild_icon_url IS NOT NULL
         GROUP BY guild_icon_url
         ORDER BY MIN(first_seen_at), guild_icon_url;",
    )
    .bind(server_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let channels = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT c.channel_id, c.channel_name, COUNT(m.message_id)
         FROM channels c
         LEFT JOIN messages m ON m.channel_id = c.channel_id
         WHERE c.guild_id = $1 AND c.channel_id = ANY($2)
         GROUP BY c.channel_id, c.channel_name
         ORDER BY c.channel_name, c.channel_id;",
    )
    .bind(server_id)
    .bind(&user.channel_ids)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let users = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT u.discord_id, u.discord_username, COUNT(m.message_id)
         FROM guild_users gu
         JOIN discord_users u ON u.discord_id = gu.discord_id
         JOIN messages m ON m.guild_id = gu.guild_id AND m.author_id = gu.discord_id
         WHERE gu.guild_id = $1 AND m.channel_id = ANY($2)
         GROUP BY u.discord_id, u.discord_username
         ORDER BY u.discord_username, u.discord_id;",
    )
    .bind(server_id)
    .bind(&user.channel_ids)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;

    let body = render_template(&ServerStatusTemplate {
        server_name: &server_name,
        server_id,
        message_count: stats.0,
        user_count: stats.1,
        channel_count: stats.2,
        version_count: stats.3,
        total_storage: format_bytes(stats.4 + stats.5),
        message_storage: format_bytes(stats.4),
        attachment_storage: format_bytes(stats.5),
        names: render_status_names(names),
        icons: render_status_icons(icons),
        channels: render_status_channels(channels),
        users: render_status_users(users),
    });
    Ok(Html(page(&server_name, &body)))
}

pub(super) async fn server_messages(
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
        query.page.unwrap_or(1),
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(super) async fn search_server_messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(server_id): Path<i64>,
    Form(form): Form<MessageSearchForm>,
) -> WebResult {
    render_server_messages(
        &data.pool,
        server_id,
        &form.search,
        &form.search_by,
        form.page.unwrap_or(1),
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(super) async fn render_server_messages(
    pool: &PgPool,
    server_id: i64,
    search: &str,
    search_by: &str,
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
        requested_page,
        MessageScope::Server(server_id, server_name),
        channel_ids,
        viewer_id,
    )
    .await
}

pub(super) async fn channel_messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(channel_id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_channel_messages(
        &data.pool,
        channel_id,
        "",
        "content",
        query.page.unwrap_or(1),
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(super) async fn search_channel_messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(channel_id): Path<i64>,
    Form(form): Form<MessageSearchForm>,
) -> WebResult {
    render_channel_messages(
        &data.pool,
        channel_id,
        &form.search,
        &form.search_by,
        form.page.unwrap_or(1),
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(super) async fn render_channel_messages(
    pool: &PgPool,
    channel_id: i64,
    search: &str,
    search_by: &str,
    requested_page: i64,
    channel_ids: &[i64],
    viewer_id: i64,
) -> WebResult {
    let channel_name =
        sqlx::query_scalar::<_, String>("SELECT channel_name FROM channels WHERE channel_id = $1;")
            .bind(channel_id)
            .fetch_optional(pool)
            .await
            .map_err(database_error)?
            .ok_or_else(|| not_found("Channel not found."))?;
    render_messages(
        pool,
        search,
        search_by,
        requested_page,
        MessageScope::Channel(channel_id, channel_name),
        channel_ids,
        viewer_id,
    )
    .await
}

pub(super) async fn render_servers(
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
        "SELECT guild_id, guild_name, guild_icon_url
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

pub(super) async fn channels(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_channels(&data.pool, "", query.page.unwrap_or(1), &user.channel_ids).await
}

pub(super) async fn search_channels(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Form(form): Form<SearchForm>,
) -> WebResult {
    render_channels(
        &data.pool,
        &form.search,
        form.page.unwrap_or(1),
        &user.channel_ids,
    )
    .await
}

pub(super) async fn channel(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(channel_id): Path<i64>,
) -> WebResult {
    let channel = sqlx::query_as::<_, (String, i64, String)>(
        "SELECT c.channel_name, g.guild_id, g.guild_name
         FROM channels c
         JOIN guilds g ON g.guild_id = c.guild_id
         WHERE c.channel_id = $1;",
    )
    .bind(channel_id)
    .fetch_optional(&data.pool)
    .await
    .map_err(database_error)?
    .ok_or_else(|| not_found("Channel not found."))?;
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(
        "SELECT
             (SELECT COUNT(*) FROM messages WHERE channel_id = $1),
             (SELECT COUNT(DISTINCT author_id) FROM messages WHERE channel_id = $1),
             (SELECT COUNT(*)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.channel_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(v.content)), 0)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.channel_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(a.data)), 0)
              FROM attachments a
              JOIN messages m ON m.message_id = a.message_id
              WHERE m.channel_id = $1);",
    )
    .bind(channel_id)
    .fetch_one(&data.pool)
    .await
    .map_err(database_error)?;
    let names = sqlx::query_as::<_, (String, String, String)>(
        "SELECT channel_name, first_seen_at::text, last_seen_at::text
         FROM channel_history
         WHERE channel_id = $1
         ORDER BY first_seen_at, channel_name;",
    )
    .bind(channel_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let users = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT u.discord_id, u.discord_username, COUNT(m.message_id)
         FROM messages m
         JOIN discord_users u ON u.discord_id = m.author_id
         WHERE m.channel_id = $1
         GROUP BY u.discord_id, u.discord_username
         ORDER BY u.discord_username, u.discord_id;",
    )
    .bind(channel_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;

    let body = render_template(&ChannelStatusTemplate {
        channel_name: &channel.0,
        channel_id,
        server_id: channel.1,
        server_name: &channel.2,
        message_count: stats.0,
        user_count: stats.1,
        version_count: stats.2,
        total_storage: format_bytes(stats.3 + stats.4),
        message_storage: format_bytes(stats.3),
        attachment_storage: format_bytes(stats.4),
        names: render_status_names(names),
        users: render_status_users(users),
        access_reason: user.access_reason(channel_id),
    });
    Ok(Html(page(&channel.0, &body)))
}

pub(super) async fn render_channels(
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

pub(super) async fn users(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_users(&data.pool, "", query.page.unwrap_or(1), &user.channel_ids).await
}

pub(super) async fn search_users(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Form(form): Form<SearchForm>,
) -> WebResult {
    render_users(
        &data.pool,
        &form.search,
        form.page.unwrap_or(1),
        &user.channel_ids,
    )
    .await
}

pub(super) async fn user(
    State(data): State<WebData>,
    Extension(web_user): Extension<WebUser>,
    Path(user_id): Path<i64>,
) -> WebResult {
    let username = sqlx::query_scalar::<_, String>(
        "SELECT discord_username FROM discord_users WHERE discord_id = $1;",
    )
    .bind(user_id)
    .fetch_optional(&data.pool)
    .await
    .map_err(database_error)?
    .ok_or_else(|| not_found("User not found."))?;
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(
        "SELECT
             (SELECT COUNT(*) FROM messages WHERE author_id = $1),
             (SELECT COUNT(*) FROM guild_users WHERE discord_id = $1),
             (SELECT COUNT(DISTINCT channel_id) FROM messages WHERE author_id = $1),
             (SELECT COUNT(*)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.author_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(v.content)), 0)
              FROM message_versions v
              JOIN messages m ON m.message_id = v.message_id
              WHERE m.author_id = $1),
             (SELECT COALESCE(SUM(OCTET_LENGTH(a.data)), 0)
              FROM attachments a
              JOIN messages m ON m.message_id = a.message_id
              WHERE m.author_id = $1);",
    )
    .bind(user_id)
    .fetch_one(&data.pool)
    .await
    .map_err(database_error)?;
    let names = sqlx::query_as::<_, (String, String, String)>(
        "SELECT discord_username, MIN(first_seen_at)::text, MAX(last_seen_at)::text
         FROM discord_user_history
         WHERE discord_id = $1
         GROUP BY discord_username
         ORDER BY MIN(first_seen_at), discord_username;",
    )
    .bind(user_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let avatars = sqlx::query_as::<_, (String, String, String)>(
        "SELECT discord_avatar_url, MIN(first_seen_at)::text, MAX(last_seen_at)::text
         FROM discord_user_history
         WHERE discord_id = $1 AND discord_avatar_url IS NOT NULL
         GROUP BY discord_avatar_url
         ORDER BY MIN(first_seen_at), discord_avatar_url;",
    )
    .bind(user_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let servers = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT g.guild_id, g.guild_name, COUNT(m.message_id)
         FROM guild_users gu
         JOIN guilds g ON g.guild_id = gu.guild_id
         JOIN messages m ON m.guild_id = gu.guild_id AND m.author_id = gu.discord_id
         WHERE gu.discord_id = $1 AND m.channel_id = ANY($2)
         GROUP BY g.guild_id, g.guild_name
         ORDER BY g.guild_name, g.guild_id;",
    )
    .bind(user_id)
    .bind(&web_user.channel_ids)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let channels = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT c.channel_id, c.channel_name, COUNT(m.message_id)
         FROM messages m
         JOIN channels c ON c.channel_id = m.channel_id
         WHERE m.author_id = $1 AND m.channel_id = ANY($2)
         GROUP BY c.channel_id, c.channel_name
         ORDER BY c.channel_name, c.channel_id;",
    )
    .bind(user_id)
    .bind(&web_user.channel_ids)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let access_reason = channels
        .first()
        .map(|channel| web_user.access_reason(channel.0))
        .unwrap_or("you can see archived messages from them");

    let body = render_template(&UserStatusTemplate {
        username: &username,
        user_id,
        message_count: stats.0,
        server_count: stats.1,
        channel_count: stats.2,
        version_count: stats.3,
        total_storage: format_bytes(stats.4 + stats.5),
        message_storage: format_bytes(stats.4),
        attachment_storage: format_bytes(stats.5),
        names: render_status_names(names),
        avatars: render_status_avatars(avatars),
        servers: render_status_servers(servers),
        channels: render_status_channels(channels),
        access_reason,
    });
    Ok(Html(page(&username, &body)))
}

pub(super) async fn user_messages(
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
        query.page.unwrap_or(1),
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(super) async fn search_user_messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(user_id): Path<i64>,
    Form(form): Form<MessageSearchForm>,
) -> WebResult {
    render_user_messages(
        &data.pool,
        user_id,
        &form.search,
        &form.search_by,
        form.page.unwrap_or(1),
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(super) async fn render_user_messages(
    pool: &PgPool,
    user_id: i64,
    search: &str,
    search_by: &str,
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
        requested_page,
        MessageScope::User(user_id, username),
        channel_ids,
        viewer_id,
    )
    .await
}

pub(super) async fn render_users(
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

pub(super) async fn messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_messages(
        &data.pool,
        "",
        "content",
        query.page.unwrap_or(1),
        MessageScope::All,
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(super) async fn search_messages(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Form(form): Form<MessageSearchForm>,
) -> WebResult {
    render_messages(
        &data.pool,
        &form.search,
        &form.search_by,
        form.page.unwrap_or(1),
        MessageScope::All,
        &user.channel_ids,
        user.id,
    )
    .await
}

pub(super) async fn render_messages(
    pool: &PgPool,
    search: &str,
    search_by: &str,
    requested_page: i64,
    scope: MessageScope,
    channel_ids: &[i64],
    viewer_id: i64,
) -> WebResult {
    let search_by = if search_by == "timestamp" {
        "timestamp"
    } else {
        "content"
    };
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

pub(super) async fn message(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(message_id): Path<i64>,
    Query(query): Query<MessageQuery>,
) -> WebResult {
    if message_id.len() = 0 {
        return Err(bad_request("Please provide a message id."));
    }
    let message = sqlx::query_as::<_, (String, i64, i64, String, i64, String, String, bool)>(
        "SELECT
             m.author_username,
             m.author_id,
             m.guild_id,
             g.guild_name,
             m.channel_id,
             c.channel_name,
             m.timestamp::text,
             m.archive_incomplete
         FROM messages m
         JOIN guilds g ON g.guild_id = m.guild_id
         JOIN channels c ON c.channel_id = m.channel_id
         WHERE m.message_id = $1;",
    )
    .bind(message_id)
    .fetch_optional(&data.pool)
    .await
    .map_err(database_error)?;
    let Some((author, author_id, server_id, server, channel_id, channel, timestamp, archive_incomplete)) = message
    else {
        return Err(not_found("Message not found."));
    };
    let timestamp = localize_timestamp(&timestamp);

    let version_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM message_versions WHERE message_id = $1;",
    )
    .bind(message_id)
    .fetch_one(&data.pool)
    .await
    .map_err(database_error)?;
    let version = sqlx::query_as::<_, (i64, Option<String>, String)>(
        "SELECT version, content, archived_at::text
         FROM message_versions
         WHERE message_id = $1 AND ($2::bigint IS NULL OR version = $2)
         ORDER BY version DESC
         LIMIT 1;",
    )
    .bind(message_id)
    .bind(query.version)
    .fetch_optional(&data.pool)
    .await
    .map_err(database_error)?
    .ok_or_else(|| not_found("Message version not found."))?;
    let (message_version, content, archived_at) = version;

    let attachments = sqlx::query_as::<_, (i64, String, Option<String>, Option<String>, i64)>(
        "SELECT attachment_id, filename, description, content_type, size
         FROM attachments
         WHERE message_id = $1 AND message_version = $2
         ORDER BY attachment_id;",
    )
    .bind(message_id)
    .bind(message_version)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let embeds = sqlx::query_as::<_, (i32, Option<String>, Option<String>, Option<String>)>(
        "SELECT embed_index, title, description, url
         FROM embeds
         WHERE message_id = $1 AND message_version = $2
         ORDER BY embed_index;",
    )
    .bind(message_id)
    .bind(message_version)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;

    let mut rendered_attachments = String::new();
    for (attachment_id, filename, description, content_type, size) in attachments {
        rendered_attachments.push_str(&render_template(&AttachmentTemplate {
            attachment_id,
            message_version,
            filename: &filename,
            size,
            content_type: content_type.as_deref(),
            description: description.as_deref(),
        }));
    }

    let mut rendered_embeds = String::new();
    for (_embed_index, title, description, url) in embeds {
        rendered_embeds.push_str(&render_template(&EmbedTemplate {
            title: title.as_deref(),
            description: description.as_deref(),
            url: url.as_deref(),
        }));
    }

    let body = render_template(&MessageTemplate {
        message_id,
        author: &author,
        author_id,
        server_id,
        server: &server,
        channel_id,
        channel: &channel,
        timestamp: &timestamp,
        archive_incomplete,
        version_navigation: message_version_navigation(
            message_id,
            message_version,
            version_count,
            &archived_at,
        ),
        content: content.as_deref().unwrap_or(""),
        attachments: rendered_attachments,
        embeds: rendered_embeds,
        access_reason: if author_id == user.id && !user.channel_ids.contains(&channel_id) {
            "you sent it"
        } else {
            user.access_reason(channel_id)
        },
    });
    Ok(Html(page(&format!("Message {}", message_id), &body)))
}

pub(super) async fn attachment(
    State(data): State<WebData>,
    Path(attachment_id): Path<i64>,
    Query(query): Query<AttachmentQuery>,
) -> Response {
    let attachment = sqlx::query_as::<_, (String, Option<String>, Vec<u8>)>(
        "SELECT filename, content_type, data
         FROM attachments
         WHERE attachment_id = $1 AND ($2::bigint IS NULL OR message_version = $2)
         ORDER BY message_version DESC
         LIMIT 1;",
    )
    .bind(attachment_id)
    .bind(query.version)
    .fetch_optional(&data.pool)
    .await;

    match attachment {
        Ok(Some((filename, content_type, data))) => {
            let content_type = content_type.unwrap_or_else(|| "application/octet-stream".into());
            let disposition = if is_inline_content_type(&content_type) {
                "inline"
            } else {
                "attachment"
            };
            (
                [
                    (header::CONTENT_TYPE, content_type),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("{}; filename=\"{}\"", disposition, safe_filename(&filename)),
                    ),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
                    (
                        header::CONTENT_SECURITY_POLICY,
                        "sandbox; default-src 'none'; img-src 'self' data:; media-src 'self'"
                            .into(),
                    ),
                ],
                data,
            )
                .into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            error!("Database error while loading attachment: {}", error);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
