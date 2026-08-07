use axum::{
    Form, Router,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tracing::{error, info};

const ITEMS_PER_PAGE: i64 = 100;
const SHOWN_PAGES: i64 = 10;

const PAGE_TEMPLATE: &str = include_str!("../static/page.html");
const INDEX_TEMPLATE: &str = include_str!("../static/index.html");
const LIST_TEMPLATE: &str = include_str!("../static/list.html");
const SEARCH_FORM_TEMPLATE: &str = include_str!("../static/search-form.html");
const MESSAGE_SEARCH_FORM_TEMPLATE: &str = include_str!("../static/message-search-form.html");
const SERVER_TEMPLATE: &str = include_str!("../static/server.html");
const SERVER_ICON_TEMPLATE: &str = include_str!("../static/server-icon.html");
const CHANNEL_TEMPLATE: &str = include_str!("../static/channel.html");
const USER_TEMPLATE: &str = include_str!("../static/user.html");
const SERVER_STATUS_TEMPLATE: &str = include_str!("../static/server-status.html");
const CHANNEL_STATUS_TEMPLATE: &str = include_str!("../static/channel-status.html");
const USER_STATUS_TEMPLATE: &str = include_str!("../static/user-status.html");
const STATUS_NAME_TEMPLATE: &str = include_str!("../static/status-name.html");
const STATUS_ICON_TEMPLATE: &str = include_str!("../static/status-icon.html");
const STATUS_AVATAR_TEMPLATE: &str = include_str!("../static/status-avatar.html");
const STATUS_CHANNEL_TEMPLATE: &str = include_str!("../static/status-channel.html");
const STATUS_SERVER_TEMPLATE: &str = include_str!("../static/status-server.html");
const STATUS_USER_TEMPLATE: &str = include_str!("../static/status-user.html");
const MESSAGE_LIST_TEMPLATE: &str = include_str!("../static/message-list.html");
const MESSAGE_LIST_ITEM_TEMPLATE: &str = include_str!("../static/message-list-item.html");
const MESSAGE_TEMPLATE: &str = include_str!("../static/message.html");
const MESSAGE_VERSION_NAVIGATION_TEMPLATE: &str =
    include_str!("../static/message-version-navigation.html");
const MESSAGE_VERSION_BUTTON_TEMPLATE: &str = include_str!("../static/message-version-button.html");
const ATTACHMENT_TEMPLATE: &str = include_str!("../static/attachment.html");
const ATTACHMENT_VALUE_TEMPLATE: &str = include_str!("../static/attachment-value.html");
const EMBED_TEMPLATE: &str = include_str!("../static/embed.html");
const EMBED_TITLE_TEMPLATE: &str = include_str!("../static/embed-title.html");
const EMBED_DESCRIPTION_TEMPLATE: &str = include_str!("../static/embed-description.html");
const EMBED_URL_TEMPLATE: &str = include_str!("../static/embed-url.html");
const PAGINATION_TEMPLATE: &str = include_str!("../static/pagination.html");
const PAGE_BUTTON_TEMPLATE: &str = include_str!("../static/page-button.html");
const CURRENT_PAGE_TEMPLATE: &str = include_str!("../static/current-page.html");
const ARBITRARY_PAGE_TEMPLATE: &str = include_str!("../static/arbitrary-page.html");
const ERROR_TEMPLATE: &str = include_str!("../static/error.html");

#[derive(Clone)]
struct WebData {
    pool: PgPool,
}

#[derive(Default, Deserialize)]
struct PageQuery {
    page: Option<i64>,
}

#[derive(Default, Deserialize)]
struct MessageQuery {
    version: Option<i64>,
}

#[derive(Default, Deserialize)]
struct AttachmentQuery {
    version: Option<i64>,
}

struct ArchiveStats {
    messages: i64,
    users: i64,
    servers: i64,
    channels: i64,
    total_storage: i64,
    message_storage: i64,
    attachment_storage: i64,
}

#[derive(Default, Deserialize)]
struct SearchForm {
    #[serde(default)]
    search: String,
    page: Option<i64>,
}

#[derive(Default, Deserialize)]
struct MessageSearchForm {
    #[serde(default)]
    search: String,
    #[serde(default)]
    search_by: String,
    page: Option<i64>,
}

enum MessageScope {
    All,
    Server(i64, String),
    Channel(i64, String),
    User(i64, String),
}

impl MessageScope {
    fn server_id(&self) -> Option<i64> {
        match self {
            Self::Server(server_id, _) => Some(*server_id),
            _ => None,
        }
    }

    fn user_id(&self) -> Option<i64> {
        match self {
            Self::User(user_id, _) => Some(*user_id),
            _ => None,
        }
    }

    fn channel_id(&self) -> Option<i64> {
        match self {
            Self::Channel(channel_id, _) => Some(*channel_id),
            _ => None,
        }
    }

    fn title(&self) -> String {
        match self {
            Self::All => "Messages".into(),
            Self::Server(_, server_name) => format!("Messages in {}", server_name),
            Self::Channel(_, channel_name) => format!("Messages in {}", channel_name),
            Self::User(_, username) => format!("Messages by {}", username),
        }
    }

    fn action(&self) -> String {
        match self {
            Self::All => "/messages".into(),
            Self::Server(server_id, _) => format!("/servers/{}/messages", server_id),
            Self::Channel(channel_id, _) => format!("/channels/{}/messages", channel_id),
            Self::User(user_id, _) => format!("/users/{}/messages", user_id),
        }
    }
}

type WebResult = Result<Html<String>, (StatusCode, Html<String>)>;

pub async fn run(listener: tokio::net::TcpListener, pool: PgPool) {
    let app = Router::new()
        .route("/", get(index))
        .route("/servers", get(servers).post(search_servers))
        .route("/servers/{server_id}", get(server))
        .route(
            "/servers/{server_id}/messages",
            get(server_messages).post(search_server_messages),
        )
        .route("/channels", get(channels).post(search_channels))
        .route("/channels/{channel_id}", get(channel))
        .route(
            "/channels/{channel_id}/messages",
            get(channel_messages).post(search_channel_messages),
        )
        .route("/users", get(users).post(search_users))
        .route("/users/{user_id}", get(user))
        .route(
            "/users/{user_id}/messages",
            get(user_messages).post(search_user_messages),
        )
        .route("/messages", get(messages).post(search_messages))
        .route("/messages/{message_id}", get(message))
        .route("/attachments/{attachment_id}", get(attachment))
        .with_state(WebData { pool });

    info!("Web server listening on {}", listener.local_addr().unwrap());
    if let Err(error) = axum::serve(listener, app).await {
        error!("Web server error: {}", error);
    }
}

async fn index(State(data): State<WebData>) -> WebResult {
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64)>(
        "SELECT
             (SELECT COUNT(*) FROM messages),
             (SELECT COUNT(*) FROM discord_users),
             (SELECT COUNT(*) FROM guilds),
             (SELECT COUNT(*) FROM channels),
             pg_total_relation_size('guilds')
                 + pg_total_relation_size('channels')
                 + pg_total_relation_size('discord_users')
                 + pg_total_relation_size('discord_roles')
                 + pg_total_relation_size('messages')
                 + pg_total_relation_size('message_versions')
                 + pg_total_relation_size('attachments')
                 + pg_total_relation_size('embeds')
                 + pg_total_relation_size('embed_fields')
                 + pg_total_relation_size('guild_history')
                 + pg_total_relation_size('channel_history')
                 + pg_total_relation_size('discord_user_history')
                 + pg_total_relation_size('guild_users'),
             pg_total_relation_size('messages')
                 + pg_total_relation_size('message_versions')
                 + pg_total_relation_size('guilds')
                 + pg_total_relation_size('channels')
                 + pg_total_relation_size('discord_users')
                 + pg_total_relation_size('discord_roles')
                 + pg_total_relation_size('embeds')
                 + pg_total_relation_size('embed_fields')
                 + pg_total_relation_size('guild_history')
                 + pg_total_relation_size('channel_history')
                 + pg_total_relation_size('discord_user_history')
                 + pg_total_relation_size('guild_users'),
             pg_total_relation_size('attachments');",
    )
    .fetch_one(&data.pool)
    .await
    .map_err(database_error)?;
    let stats = ArchiveStats {
        messages: stats.0,
        users: stats.1,
        servers: stats.2,
        channels: stats.3,
        total_storage: stats.4,
        message_storage: stats.5,
        attachment_storage: stats.6,
    };
    let body = INDEX_TEMPLATE
        .replace("$${{message_count}}", &stats.messages.to_string())
        .replace("$${{user_count}}", &stats.users.to_string())
        .replace("$${{server_count}}", &stats.servers.to_string())
        .replace("$${{channel_count}}", &stats.channels.to_string())
        .replace("$${{total_storage}}", &format_bytes(stats.total_storage))
        .replace(
            "$${{message_storage}}",
            &format_bytes(stats.message_storage),
        )
        .replace(
            "$${{attachment_storage}}",
            &format_bytes(stats.attachment_storage),
        );
    Ok(Html(page("Archive", &body)))
}

async fn servers(State(data): State<WebData>, Query(query): Query<PageQuery>) -> WebResult {
    render_servers(&data.pool, "", query.page.unwrap_or(1)).await
}

async fn search_servers(State(data): State<WebData>, Form(form): Form<SearchForm>) -> WebResult {
    render_servers(&data.pool, &form.search, form.page.unwrap_or(1)).await
}

async fn server(State(data): State<WebData>, Path(server_id): Path<i64>) -> WebResult {
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
         WHERE c.guild_id = $1
         GROUP BY c.channel_id, c.channel_name
         ORDER BY c.channel_name, c.channel_id;",
    )
    .bind(server_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let users = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT u.discord_id, u.discord_username, COUNT(m.message_id)
         FROM guild_users gu
         JOIN discord_users u ON u.discord_id = gu.discord_id
         LEFT JOIN messages m ON m.guild_id = gu.guild_id AND m.author_id = gu.discord_id
         WHERE gu.guild_id = $1
         GROUP BY u.discord_id, u.discord_username
         ORDER BY u.discord_username, u.discord_id;",
    )
    .bind(server_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;

    let body = SERVER_STATUS_TEMPLATE
        .replace("$${{server_name}}", &escape_html(&server_name))
        .replace("$${{server_id}}", &server_id.to_string())
        .replace("$${{message_count}}", &stats.0.to_string())
        .replace("$${{user_count}}", &stats.1.to_string())
        .replace("$${{channel_count}}", &stats.2.to_string())
        .replace("$${{version_count}}", &stats.3.to_string())
        .replace("$${{total_storage}}", &format_bytes(stats.4 + stats.5))
        .replace("$${{message_storage}}", &format_bytes(stats.4))
        .replace("$${{attachment_storage}}", &format_bytes(stats.5))
        .replace("$${{names}}", &render_status_names(names))
        .replace("$${{icons}}", &render_status_icons(icons))
        .replace("$${{channels}}", &render_status_channels(channels))
        .replace("$${{users}}", &render_status_users(users));
    Ok(Html(page(&server_name, &body)))
}

async fn server_messages(
    State(data): State<WebData>,
    Path(server_id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_server_messages(
        &data.pool,
        server_id,
        "",
        "content",
        query.page.unwrap_or(1),
    )
    .await
}

async fn search_server_messages(
    State(data): State<WebData>,
    Path(server_id): Path<i64>,
    Form(form): Form<MessageSearchForm>,
) -> WebResult {
    render_server_messages(
        &data.pool,
        server_id,
        &form.search,
        &form.search_by,
        form.page.unwrap_or(1),
    )
    .await
}

async fn render_server_messages(
    pool: &PgPool,
    server_id: i64,
    search: &str,
    search_by: &str,
    requested_page: i64,
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
    )
    .await
}

async fn channel_messages(
    State(data): State<WebData>,
    Path(channel_id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_channel_messages(
        &data.pool,
        channel_id,
        "",
        "content",
        query.page.unwrap_or(1),
    )
    .await
}

async fn search_channel_messages(
    State(data): State<WebData>,
    Path(channel_id): Path<i64>,
    Form(form): Form<MessageSearchForm>,
) -> WebResult {
    render_channel_messages(
        &data.pool,
        channel_id,
        &form.search,
        &form.search_by,
        form.page.unwrap_or(1),
    )
    .await
}

async fn render_channel_messages(
    pool: &PgPool,
    channel_id: i64,
    search: &str,
    search_by: &str,
    requested_page: i64,
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
    )
    .await
}

async fn render_servers(pool: &PgPool, search: &str, requested_page: i64) -> WebResult {
    let search_pattern = format!("%{}%", search);
    let item_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM guilds
         WHERE guild_name ILIKE $1 OR guild_id::text ILIKE $1;",
    )
    .bind(&search_pattern)
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    let pagination = Pagination::new(requested_page, item_count);
    let servers = sqlx::query_as::<_, (i64, String, Option<String>)>(
        "SELECT guild_id, guild_name, guild_icon_url
         FROM guilds
         WHERE guild_name ILIKE $1 OR guild_id::text ILIKE $1
         ORDER BY guild_name, guild_id
         LIMIT $2 OFFSET $3;",
    )
    .bind(&search_pattern)
    .bind(ITEMS_PER_PAGE)
    .bind(pagination.offset())
    .fetch_all(pool)
    .await
    .map_err(database_error)?;

    let mut items = String::new();
    for (guild_id, guild_name, guild_icon_url) in servers {
        let icon = guild_icon_url
            .map(|icon_url| SERVER_ICON_TEMPLATE.replace("$${{icon_url}}", &escape_html(&icon_url)))
            .unwrap_or_default();
        items.push_str(
            &SERVER_TEMPLATE
                .replace("$${{guild_name}}", &escape_html(&guild_name))
                .replace("$${{guild_id}}", &guild_id.to_string())
                .replace("$${{icon}}", &icon),
        );
    }

    let body = LIST_TEMPLATE
        .replace("$${{title}}", "Servers")
        .replace(
            "$${{search_form}}",
            &search_form("/servers", search, "Search servers"),
        )
        .replace("$${{item_count}}", &item_count.to_string())
        .replace("$${{item_name}}", "archived servers")
        .replace("$${{items}}", &items)
        .replace("$${{pagination}}", &pagination.render("/servers", search));
    Ok(Html(page("Servers", &body)))
}

async fn channels(State(data): State<WebData>, Query(query): Query<PageQuery>) -> WebResult {
    render_channels(&data.pool, "", query.page.unwrap_or(1)).await
}

async fn search_channels(State(data): State<WebData>, Form(form): Form<SearchForm>) -> WebResult {
    render_channels(&data.pool, &form.search, form.page.unwrap_or(1)).await
}

async fn channel(State(data): State<WebData>, Path(channel_id): Path<i64>) -> WebResult {
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

    let body = CHANNEL_STATUS_TEMPLATE
        .replace("$${{channel_name}}", &escape_html(&channel.0))
        .replace("$${{channel_id}}", &channel_id.to_string())
        .replace("$${{server_id}}", &channel.1.to_string())
        .replace("$${{server_name}}", &escape_html(&channel.2))
        .replace("$${{message_count}}", &stats.0.to_string())
        .replace("$${{user_count}}", &stats.1.to_string())
        .replace("$${{version_count}}", &stats.2.to_string())
        .replace("$${{total_storage}}", &format_bytes(stats.3 + stats.4))
        .replace("$${{message_storage}}", &format_bytes(stats.3))
        .replace("$${{attachment_storage}}", &format_bytes(stats.4))
        .replace("$${{names}}", &render_status_names(names))
        .replace("$${{users}}", &render_status_users(users));
    Ok(Html(page(&channel.0, &body)))
}

async fn render_channels(pool: &PgPool, search: &str, requested_page: i64) -> WebResult {
    let search_pattern = format!("%{}%", search);
    let item_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM channels c
         JOIN guilds g ON g.guild_id = c.guild_id
         WHERE c.channel_name ILIKE $1
            OR c.channel_id::text ILIKE $1
            OR g.guild_name ILIKE $1;",
    )
    .bind(&search_pattern)
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    let pagination = Pagination::new(requested_page, item_count);
    let channels = sqlx::query_as::<_, (i64, String, i64, String)>(
        "SELECT c.channel_id, c.channel_name, g.guild_id, g.guild_name
         FROM channels c
         JOIN guilds g ON g.guild_id = c.guild_id
         WHERE c.channel_name ILIKE $1
            OR c.channel_id::text ILIKE $1
            OR g.guild_name ILIKE $1
         ORDER BY c.channel_name, c.channel_id
         LIMIT $2 OFFSET $3;",
    )
    .bind(&search_pattern)
    .bind(ITEMS_PER_PAGE)
    .bind(pagination.offset())
    .fetch_all(pool)
    .await
    .map_err(database_error)?;

    let mut items = String::new();
    for (channel_id, channel_name, server_id, server_name) in channels {
        items.push_str(
            &CHANNEL_TEMPLATE
                .replace("$${{channel_id}}", &channel_id.to_string())
                .replace("$${{channel_name}}", &escape_html(&channel_name))
                .replace("$${{server_id}}", &server_id.to_string())
                .replace("$${{server_name}}", &escape_html(&server_name)),
        );
    }

    let body = LIST_TEMPLATE
        .replace("$${{title}}", "Channels")
        .replace(
            "$${{search_form}}",
            &search_form("/channels", search, "Search channels"),
        )
        .replace("$${{item_count}}", &item_count.to_string())
        .replace("$${{item_name}}", "archived channels")
        .replace("$${{items}}", &items)
        .replace("$${{pagination}}", &pagination.render("/channels", search));
    Ok(Html(page("Channels", &body)))
}

async fn users(State(data): State<WebData>, Query(query): Query<PageQuery>) -> WebResult {
    render_users(&data.pool, "", query.page.unwrap_or(1)).await
}

async fn search_users(State(data): State<WebData>, Form(form): Form<SearchForm>) -> WebResult {
    render_users(&data.pool, &form.search, form.page.unwrap_or(1)).await
}

async fn user(State(data): State<WebData>, Path(user_id): Path<i64>) -> WebResult {
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
         LEFT JOIN messages m ON m.guild_id = gu.guild_id AND m.author_id = gu.discord_id
         WHERE gu.discord_id = $1
         GROUP BY g.guild_id, g.guild_name
         ORDER BY g.guild_name, g.guild_id;",
    )
    .bind(user_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let channels = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT c.channel_id, c.channel_name, COUNT(m.message_id)
         FROM messages m
         JOIN channels c ON c.channel_id = m.channel_id
         WHERE m.author_id = $1
         GROUP BY c.channel_id, c.channel_name
         ORDER BY c.channel_name, c.channel_id;",
    )
    .bind(user_id)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;

    let body = USER_STATUS_TEMPLATE
        .replace("$${{username}}", &escape_html(&username))
        .replace("$${{user_id}}", &user_id.to_string())
        .replace("$${{message_count}}", &stats.0.to_string())
        .replace("$${{server_count}}", &stats.1.to_string())
        .replace("$${{channel_count}}", &stats.2.to_string())
        .replace("$${{version_count}}", &stats.3.to_string())
        .replace("$${{total_storage}}", &format_bytes(stats.4 + stats.5))
        .replace("$${{message_storage}}", &format_bytes(stats.4))
        .replace("$${{attachment_storage}}", &format_bytes(stats.5))
        .replace("$${{names}}", &render_status_names(names))
        .replace("$${{avatars}}", &render_status_avatars(avatars))
        .replace("$${{servers}}", &render_status_servers(servers))
        .replace("$${{channels}}", &render_status_channels(channels));
    Ok(Html(page(&username, &body)))
}

async fn user_messages(
    State(data): State<WebData>,
    Path(user_id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_user_messages(&data.pool, user_id, "", "content", query.page.unwrap_or(1)).await
}

async fn search_user_messages(
    State(data): State<WebData>,
    Path(user_id): Path<i64>,
    Form(form): Form<MessageSearchForm>,
) -> WebResult {
    render_user_messages(
        &data.pool,
        user_id,
        &form.search,
        &form.search_by,
        form.page.unwrap_or(1),
    )
    .await
}

async fn render_user_messages(
    pool: &PgPool,
    user_id: i64,
    search: &str,
    search_by: &str,
    requested_page: i64,
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
    )
    .await
}

async fn render_users(pool: &PgPool, search: &str, requested_page: i64) -> WebResult {
    let search_pattern = format!("%{}%", search);
    let item_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM discord_users
         WHERE discord_username ILIKE $1 OR discord_id::text ILIKE $1;",
    )
    .bind(&search_pattern)
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    let pagination = Pagination::new(requested_page, item_count);
    let users = sqlx::query_as::<_, (i64, String)>(
        "SELECT discord_id, discord_username
         FROM discord_users
         WHERE discord_username ILIKE $1 OR discord_id::text ILIKE $1
         ORDER BY discord_username, discord_id
         LIMIT $2 OFFSET $3;",
    )
    .bind(&search_pattern)
    .bind(ITEMS_PER_PAGE)
    .bind(pagination.offset())
    .fetch_all(pool)
    .await
    .map_err(database_error)?;

    let mut items = String::new();
    for (discord_id, discord_username) in users {
        items.push_str(
            &USER_TEMPLATE
                .replace("$${{discord_username}}", &escape_html(&discord_username))
                .replace("$${{discord_id}}", &discord_id.to_string()),
        );
    }

    let body = LIST_TEMPLATE
        .replace("$${{title}}", "Users")
        .replace(
            "$${{search_form}}",
            &search_form("/users", search, "Search users"),
        )
        .replace("$${{item_count}}", &item_count.to_string())
        .replace("$${{item_name}}", "archived users")
        .replace("$${{items}}", &items)
        .replace("$${{pagination}}", &pagination.render("/users", search));
    Ok(Html(page("Users", &body)))
}

async fn messages(State(data): State<WebData>, Query(query): Query<PageQuery>) -> WebResult {
    render_messages(
        &data.pool,
        "",
        "content",
        query.page.unwrap_or(1),
        MessageScope::All,
    )
    .await
}

async fn search_messages(
    State(data): State<WebData>,
    Form(form): Form<MessageSearchForm>,
) -> WebResult {
    render_messages(
        &data.pool,
        &form.search,
        &form.search_by,
        form.page.unwrap_or(1),
        MessageScope::All,
    )
    .await
}

async fn render_messages(
    pool: &PgPool,
    search: &str,
    search_by: &str,
    requested_page: i64,
    scope: MessageScope,
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
           AND ($5::bigint IS NULL OR m.channel_id = $5);",
    )
    .bind(&search_pattern)
    .bind(search_by)
    .bind(scope.server_id())
    .bind(scope.user_id())
    .bind(scope.channel_id())
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
         ORDER BY m.timestamp DESC, m.message_id DESC
         LIMIT $6 OFFSET $7;",
        )
        .bind(&search_pattern)
        .bind(search_by)
        .bind(scope.server_id())
        .bind(scope.user_id())
        .bind(scope.channel_id())
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
        items.push_str(
            &MESSAGE_LIST_ITEM_TEMPLATE
                .replace("$${{message_id}}", &message_id.to_string())
                .replace("$${{author_id}}", &author_id.to_string())
                .replace("$${{author}}", &escape_html(&author))
                .replace("$${{server_id}}", &server_id.to_string())
                .replace("$${{server}}", &escape_html(&server))
                .replace("$${{channel_id}}", &channel_id.to_string())
                .replace("$${{channel}}", &escape_html(&channel))
                .replace("$${{timestamp}}", &escape_html(&timestamp))
                .replace("$${{attachment_count}}", &attachment_count.to_string())
                .replace("$${{embed_count}}", &embed_count.to_string()),
        );
    }

    let title = scope.title();
    let action = scope.action();
    let body = MESSAGE_LIST_TEMPLATE
        .replace("$${{title}}", &escape_html(&title))
        .replace(
            "$${{search_form}}",
            &message_search_form(&action, search, search_by),
        )
        .replace("$${{item_count}}", &item_count.to_string())
        .replace("$${{items}}", &items)
        .replace(
            "$${{pagination}}",
            &pagination.render_messages(&action, search, search_by),
        );
    Ok(Html(page(&title, &body)))
}

async fn message(
    State(data): State<WebData>,
    Path(message_id): Path<i64>,
    Query(query): Query<MessageQuery>,
) -> WebResult {
    let message = sqlx::query_as::<_, (String, i64, i64, String, i64, String, String)>(
        "SELECT
             m.author_username,
             m.author_id,
             m.guild_id,
             g.guild_name,
             m.channel_id,
             c.channel_name,
             m.timestamp::text
         FROM messages m
         JOIN guilds g ON g.guild_id = m.guild_id
         JOIN channels c ON c.channel_id = m.channel_id
         WHERE m.message_id = $1;",
    )
    .bind(message_id)
    .fetch_optional(&data.pool)
    .await
    .map_err(database_error)?;
    let Some((author, author_id, server_id, server, channel_id, channel, timestamp)) = message
    else {
        return Err(not_found("Message not found."));
    };

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
        let rendered_content_type = content_type
            .map(|value| ATTACHMENT_VALUE_TEMPLATE.replace("$${{value}}", &escape_html(&value)))
            .unwrap_or_default();
        let rendered_description = description
            .map(|value| ATTACHMENT_VALUE_TEMPLATE.replace("$${{value}}", &escape_html(&value)))
            .unwrap_or_default();
        rendered_attachments.push_str(
            &ATTACHMENT_TEMPLATE
                .replace("$${{attachment_id}}", &attachment_id.to_string())
                .replace("$${{message_version}}", &message_version.to_string())
                .replace("$${{filename}}", &escape_html(&filename))
                .replace("$${{size}}", &size.to_string())
                .replace("$${{content_type}}", &rendered_content_type)
                .replace("$${{description}}", &rendered_description),
        );
    }

    let mut rendered_embeds = String::new();
    for (_embed_index, title, description, url) in embeds {
        let rendered_title = title
            .map(|value| EMBED_TITLE_TEMPLATE.replace("$${{title}}", &escape_html(&value)))
            .unwrap_or_default();
        let rendered_description = description
            .map(|value| {
                EMBED_DESCRIPTION_TEMPLATE.replace("$${{description}}", &escape_html(&value))
            })
            .unwrap_or_default();
        let rendered_url = url
            .map(|value| EMBED_URL_TEMPLATE.replace("$${{url}}", &escape_html(&value)))
            .unwrap_or_default();
        rendered_embeds.push_str(
            &EMBED_TEMPLATE
                .replace("$${{title}}", &rendered_title)
                .replace("$${{description}}", &rendered_description)
                .replace("$${{url}}", &rendered_url),
        );
    }

    let body = MESSAGE_TEMPLATE
        .replace("$${{message_id}}", &message_id.to_string())
        .replace("$${{author}}", &escape_html(&author))
        .replace("$${{author_id}}", &author_id.to_string())
        .replace("$${{server_id}}", &server_id.to_string())
        .replace("$${{server}}", &escape_html(&server))
        .replace("$${{channel_id}}", &channel_id.to_string())
        .replace("$${{channel}}", &escape_html(&channel))
        .replace("$${{timestamp}}", &escape_html(&timestamp))
        .replace(
            "$${{version_navigation}}",
            &message_version_navigation(message_id, message_version, version_count, &archived_at),
        )
        .replace(
            "$${{content}}",
            &escape_html(content.as_deref().unwrap_or("")),
        )
        .replace("$${{attachments}}", &rendered_attachments)
        .replace("$${{embeds}}", &rendered_embeds);
    Ok(Html(page(&format!("Message {}", message_id), &body)))
}

async fn attachment(
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

struct Pagination {
    current_page: i64,
    total_pages: i64,
}

impl Pagination {
    fn new(requested_page: i64, item_count: i64) -> Self {
        let total_pages = ((item_count + ITEMS_PER_PAGE - 1) / ITEMS_PER_PAGE).max(1);
        Self {
            current_page: requested_page.clamp(1, total_pages),
            total_pages,
        }
    }

    fn offset(&self) -> i64 {
        (self.current_page - 1) * ITEMS_PER_PAGE
    }

    fn render(&self, action: &str, search: &str) -> String {
        self.render_with_fields(action, search, "")
    }

    fn render_messages(&self, action: &str, search: &str, search_by: &str) -> String {
        self.render_with_fields(
            action,
            search,
            &format!(
                "    <input type=\"hidden\" name=\"search_by\" value=\"{}\">\n",
                escape_html(search_by)
            ),
        )
    }

    fn render_with_fields(&self, action: &str, search: &str, additional_fields: &str) -> String {
        let mut first_pages = String::new();
        if self.current_page > 1 {
            first_pages.push_str(&page_button(
                action,
                search,
                additional_fields,
                self.current_page - 1,
                "&lt;",
            ));
        }
        for page_number in 1..=self.total_pages.min(SHOWN_PAGES) {
            first_pages.push_str(&self.number(action, search, additional_fields, page_number));
        }

        let mut arbitrary_page = String::new();
        let mut final_pages = String::new();
        if self.total_pages > SHOWN_PAGES {
            arbitrary_page = ARBITRARY_PAGE_TEMPLATE
                .replace("$${{action}}", action)
                .replace("$${{search}}", &escape_html(search))
                .replace("$${{additional_fields}}", additional_fields)
                .replace("$${{total_pages}}", &self.total_pages.to_string());
            for page_number in (self.total_pages - 2).max(SHOWN_PAGES + 1)..=self.total_pages {
                final_pages.push_str(&self.number(action, search, additional_fields, page_number));
            }
        }
        if self.current_page < self.total_pages {
            final_pages.push_str(&page_button(
                action,
                search,
                additional_fields,
                self.current_page + 1,
                "&gt;",
            ));
        }

        PAGINATION_TEMPLATE
            .replace("$${{first_pages}}", &first_pages)
            .replace("$${{arbitrary_page}}", &arbitrary_page)
            .replace("$${{final_pages}}", &final_pages)
            .replace("$${{current_page}}", &self.current_page.to_string())
            .replace("$${{total_pages}}", &self.total_pages.to_string())
    }

    fn number(
        &self,
        action: &str,
        search: &str,
        additional_fields: &str,
        page_number: i64,
    ) -> String {
        if page_number == self.current_page {
            CURRENT_PAGE_TEMPLATE.replace("$${{page_number}}", &page_number.to_string())
        } else {
            page_button(
                action,
                search,
                additional_fields,
                page_number,
                &page_number.to_string(),
            )
        }
    }
}

fn page_button(
    action: &str,
    search: &str,
    additional_fields: &str,
    page_number: i64,
    label: &str,
) -> String {
    PAGE_BUTTON_TEMPLATE
        .replace("$${{action}}", action)
        .replace("$${{search}}", &escape_html(search))
        .replace("$${{additional_fields}}", additional_fields)
        .replace("$${{page_number}}", &page_number.to_string())
        .replace("$${{label}}", label)
}

fn search_form(action: &str, search: &str, label: &str) -> String {
    SEARCH_FORM_TEMPLATE
        .replace("$${{action}}", action)
        .replace("$${{label}}", label)
        .replace("$${{search}}", &escape_html(search))
}

fn render_status_names(names: Vec<(String, String, String)>) -> String {
    let mut rendered_names = String::new();
    for (name, first_seen, last_seen) in names {
        rendered_names.push_str(
            &STATUS_NAME_TEMPLATE
                .replace("$${{name}}", &escape_html(&name))
                .replace("$${{first_seen}}", &escape_html(&first_seen))
                .replace("$${{last_seen}}", &escape_html(&last_seen)),
        );
    }
    rendered_names
}

fn render_status_icons(icons: Vec<(String, String, String)>) -> String {
    let mut rendered_icons = String::new();
    for (icon_url, first_seen, last_seen) in icons {
        rendered_icons.push_str(
            &STATUS_ICON_TEMPLATE
                .replace("$${{icon_url}}", &escape_html(&icon_url))
                .replace("$${{first_seen}}", &escape_html(&first_seen))
                .replace("$${{last_seen}}", &escape_html(&last_seen)),
        );
    }
    rendered_icons
}

fn render_status_avatars(avatars: Vec<(String, String, String)>) -> String {
    let mut rendered_avatars = String::new();
    for (avatar_url, first_seen, last_seen) in avatars {
        rendered_avatars.push_str(
            &STATUS_AVATAR_TEMPLATE
                .replace("$${{avatar_url}}", &escape_html(&avatar_url))
                .replace("$${{first_seen}}", &escape_html(&first_seen))
                .replace("$${{last_seen}}", &escape_html(&last_seen)),
        );
    }
    rendered_avatars
}

fn render_status_channels(channels: Vec<(i64, String, i64)>) -> String {
    let mut rendered_channels = String::new();
    for (channel_id, channel_name, message_count) in channels {
        rendered_channels.push_str(
            &STATUS_CHANNEL_TEMPLATE
                .replace("$${{channel_id}}", &channel_id.to_string())
                .replace("$${{channel_name}}", &escape_html(&channel_name))
                .replace("$${{message_count}}", &message_count.to_string()),
        );
    }
    rendered_channels
}

fn render_status_servers(servers: Vec<(i64, String, i64)>) -> String {
    let mut rendered_servers = String::new();
    for (server_id, server_name, message_count) in servers {
        rendered_servers.push_str(
            &STATUS_SERVER_TEMPLATE
                .replace("$${{server_id}}", &server_id.to_string())
                .replace("$${{server_name}}", &escape_html(&server_name))
                .replace("$${{message_count}}", &message_count.to_string()),
        );
    }
    rendered_servers
}

fn render_status_users(users: Vec<(i64, String, i64)>) -> String {
    let mut rendered_users = String::new();
    for (user_id, username, message_count) in users {
        rendered_users.push_str(
            &STATUS_USER_TEMPLATE
                .replace("$${{user_id}}", &user_id.to_string())
                .replace("$${{username}}", &escape_html(&username))
                .replace("$${{message_count}}", &message_count.to_string()),
        );
    }
    rendered_users
}

fn message_search_form(action: &str, search: &str, search_by: &str) -> String {
    let (content_selected, timestamp_selected) = if search_by == "timestamp" {
        ("", " selected")
    } else {
        (" selected", "")
    };
    MESSAGE_SEARCH_FORM_TEMPLATE
        .replace("$${{action}}", action)
        .replace("$${{search}}", &escape_html(search))
        .replace("$${{content_selected}}", content_selected)
        .replace("$${{timestamp_selected}}", timestamp_selected)
}

fn message_version_navigation(
    message_id: i64,
    current_version: i64,
    version_count: i64,
    archived_at: &str,
) -> String {
    let previous_version = if current_version > 1 {
        message_version_button(message_id, current_version - 1, "Previous version")
    } else {
        String::new()
    };
    let next_version = if current_version < version_count {
        message_version_button(message_id, current_version + 1, "Next version")
    } else {
        String::new()
    };
    MESSAGE_VERSION_NAVIGATION_TEMPLATE
        .replace("$${{message_id}}", &message_id.to_string())
        .replace("$${{current_version}}", &current_version.to_string())
        .replace("$${{version_count}}", &version_count.to_string())
        .replace("$${{archived_at}}", &escape_html(archived_at))
        .replace("$${{previous_version}}", &previous_version)
        .replace("$${{next_version}}", &next_version)
}

fn message_version_button(message_id: i64, version: i64, label: &str) -> String {
    MESSAGE_VERSION_BUTTON_TEMPLATE
        .replace("$${{message_id}}", &message_id.to_string())
        .replace("$${{version}}", &version.to_string())
        .replace("$${{label}}", label)
}

fn page(title: &str, body: &str) -> String {
    PAGE_TEMPLATE
        .replace("$${{title}}", &escape_html(title))
        .replace("$${{body}}", body)
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn safe_filename(filename: &str) -> String {
    filename
        .chars()
        .filter(|character| !matches!(character, '\r' | '\n' | '"' | '\\'))
        .collect()
}

fn is_inline_content_type(content_type: &str) -> bool {
    let content_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        content_type.as_str(),
        "application/pdf"
            | "image/avif"
            | "image/bmp"
            | "image/gif"
            | "image/jpeg"
            | "image/png"
            | "image/webp"
            | "image/x-icon"
            | "text/plain"
    ) || content_type.starts_with("audio/")
        || content_type.starts_with("video/")
}

fn format_bytes(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];

    let mut size = bytes.max(0) as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", bytes.max(0), UNITS[unit])
    } else {
        format!("{:.1} {}", size, UNITS[unit])
    }
}

fn database_error(error: sqlx::Error) -> (StatusCode, Html<String>) {
    error!("Database error while rendering web page: {}", error);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(page(
            "Error",
            &ERROR_TEMPLATE.replace("$${{message}}", "The archive could not be loaded."),
        )),
    )
}

fn not_found(message: &str) -> (StatusCode, Html<String>) {
    (
        StatusCode::NOT_FOUND,
        Html(page(
            "Not found",
            &ERROR_TEMPLATE.replace("$${{message}}", message),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_from_the_database() {
        assert_eq!(
            escape_html("<script>'\"&</script>"),
            "&lt;script&gt;&#39;&quot;&amp;&lt;/script&gt;"
        );
    }

    #[test]
    fn renders_short_pagination_without_arbitrary_page_form() {
        let pagination = Pagination::new(2, ITEMS_PER_PAGE * 3);
        let html = pagination.render("/messages", "test");

        assert!(html.contains("Page 2 of 3"));
        assert!(!html.contains("type=\"number\""));
        assert!(!html.contains("$${{"));
    }

    #[test]
    fn renders_long_pagination_with_arbitrary_and_final_pages() {
        let pagination = Pagination::new(50, ITEMS_PER_PAGE * 100);
        let html = pagination.render("/messages", "test");

        assert!(html.contains("max=\"100\""));
        assert!(html.contains("value=\"98\""));
        assert!(html.contains("value=\"99\""));
        assert!(html.contains("value=\"100\""));
        assert!(html.contains("Page 50 of 100"));
        assert!(!html.contains("$${{"));
    }

    #[test]
    fn message_pagination_keeps_the_search_type() {
        let pagination = Pagination::new(2, ITEMS_PER_PAGE * 3);
        let html = pagination.render_messages("/servers/123", "2026-08", "timestamp");

        assert!(html.contains("action=\"/servers/123\""));
        assert!(html.contains("name=\"search\" value=\"2026-08\""));
        assert!(html.contains("name=\"search_by\" value=\"timestamp\""));
        assert!(!html.contains("$${{"));
    }

    #[test]
    fn message_search_form_selects_timestamp_and_escapes_the_search() {
        let html = message_search_form("/users/123", "<date>", "timestamp");

        assert!(html.contains("action=\"/users/123\""));
        assert!(html.contains("value=\"&lt;date&gt;\""));
        assert!(html.contains("value=\"timestamp\" selected"));
        assert!(!html.contains("value=\"content\" selected"));
        assert!(!html.contains("$${{"));
    }

    #[test]
    fn formats_archive_storage_sizes() {
        assert_eq!(format_bytes(0), "0 bytes");
        assert_eq!(format_bytes(1023), "1023 bytes");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024 * 5 / 2), "2.5 MB");
        assert_eq!(format_bytes(1024_i64.pow(4)), "1.0 TB");
    }

    #[test]
    fn only_safe_attachment_types_open_inline() {
        assert!(is_inline_content_type("image/png"));
        assert!(is_inline_content_type("video/mp4"));
        assert!(is_inline_content_type("text/plain; charset=utf-8"));
        assert!(!is_inline_content_type("text/html"));
        assert!(!is_inline_content_type("image/svg+xml"));
        assert!(!is_inline_content_type("application/octet-stream"));
    }

    #[test]
    fn renders_message_version_navigation() {
        let html = message_version_navigation(123, 2, 3, "2026-08-07 12:00:00+00");

        assert!(html.contains("Version 2 of 3"));
        assert!(html.contains("value=\"1\">Previous version"));
        assert!(html.contains("value=\"3\">Next version"));
        assert!(html.contains("max=\"3\" value=\"2\""));
        assert!(!html.contains("$${{"));
    }

    #[test]
    fn channel_scope_targets_channel_messages() {
        let scope = MessageScope::Channel(123, "general".into());

        assert_eq!(scope.channel_id(), Some(123));
        assert_eq!(scope.server_id(), None);
        assert_eq!(scope.user_id(), None);
        assert_eq!(scope.title(), "Messages in general");
        assert_eq!(scope.action(), "/channels/123/messages");
    }

    #[test]
    fn renders_and_escapes_status_history() {
        let html = render_status_names(vec![(
            "<old name>".into(),
            "2026-01-01".into(),
            "2026-02-01".into(),
        )]);

        assert!(html.contains("&lt;old name&gt;"));
        assert!(html.contains("first seen 2026-01-01"));
        assert!(!html.contains("$${{"));
    }
}
