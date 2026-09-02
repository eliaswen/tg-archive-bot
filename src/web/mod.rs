mod api;
mod auth;
mod handlers;

use askama::Template;
use axum::{
    Extension, Form, Json, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use rand::{distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use tower_sessions::{Expiry, Session, SessionManagerLayer, cookie::time::Duration};
use tower_sessions_redis_store::{RedisStore, fred::prelude::*};
use tracing::{error, info};

pub use crate::archive_stats::format_bytes;
use auth::*;
use handlers::*;

tokio::task_local! {
    static ACTIVE_THEME: Theme;
    static ACTIVE_TIMEZONE: TimezoneContext;
}

const ITEMS_PER_PAGE: i64 = 100;
const SHOWN_PAGES: i64 = 10;
const CHANNEL_ACCESS_TTL_SECONDS: u64 = 5 * 60;

#[derive(Clone)]
struct WebData {
    pool: PgPool,
    http: reqwest::Client,
    bot_token: String,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    redis: Pool,
    token_rate_limiter: api::TokenRateLimiter,
    api_permission_cache: api::ApiPermissionCache,
}

#[derive(Clone, Deserialize, Serialize)]
struct WebUser {
    id: i64,
    username: String,
    channel_ids: Vec<i64>,
    channel_access: Vec<ChannelAccess>,
    #[serde(default)]
    channel_access_refreshed_at: u64,
}

#[derive(Clone, Deserialize, Serialize)]
struct ChannelAccess {
    channel_id: i64,
    reason: String,
}

impl WebUser {
    fn access_reason(&self, channel_id: i64) -> &str {
        self.channel_access
            .iter()
            .find(|access| access.channel_id == channel_id)
            .map(|access| access.reason.as_str())
            .unwrap_or("you are a member of a server containing it")
    }
}

#[derive(Template)]
#[template(path = "page.html")]
struct PageTemplate<'a> {
    title: &'a str,
    body: &'a str,
    theme: &'a str,
    logged_in: bool,
    detect_timezone: bool,
}

#[derive(Clone)]
struct TimezoneContext {
    timezone: chrono_tz::Tz,
    detect: bool,
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate<'a> {
    error: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "privacy.html")]
struct PrivacyTemplate {
    logged_in: bool,
}

#[derive(Template)]
#[template(path = "anonymize-confirmation.html")]
struct AnonymizeConfirmationTemplate;

#[derive(Clone, Copy)]
enum Theme {
    White,
    Black,
    Oled,
}

impl Theme {
    fn from_cookie(value: Option<&str>) -> Self {
        match value {
            Some("black") => Self::Black,
            Some("oled") => Self::Oled,
            _ => Self::White,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Black => "black",
            Self::Oled => "oled",
        }
    }
}

#[derive(Deserialize)]
struct ThemeForm {
    theme: String,
}

#[derive(Deserialize)]
struct TimezoneForm {
    timezone: String,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    message_count: i64,
    user_count: i64,
    server_count: i64,
    channel_count: i64,
    total_storage: String,
    message_storage: String,
    attachment_storage: String,
}

#[derive(Template)]
#[template(path = "list.html")]
struct ListTemplate<'a> {
    title: &'a str,
    search_form: String,
    item_count: i64,
    item_name: &'a str,
    items: String,
    pagination: String,
}

#[derive(Template)]
#[template(path = "search-form.html")]
struct SearchFormTemplate<'a> {
    action: &'a str,
    label: &'a str,
    search: &'a str,
}

#[derive(Template)]
#[template(path = "server.html")]
struct ServerTemplate<'a> {
    guild_id: i64,
    guild_name: &'a str,
    icon: String,
}

#[derive(Template)]
#[template(path = "server-icon.html")]
struct ServerIconTemplate<'a> {
    icon_url: &'a str,
}

#[derive(Template)]
#[template(path = "channel.html")]
struct ChannelTemplate<'a> {
    channel_id: i64,
    channel_name: &'a str,
    server_id: i64,
    server_name: &'a str,
}

#[derive(Template)]
#[template(path = "user.html")]
struct UserTemplate<'a> {
    discord_id: i64,
    discord_username: &'a str,
}

#[derive(Template)]
#[template(path = "server-status.html")]
struct ServerStatusTemplate<'a> {
    server_name: &'a str,
    server_id: i64,
    message_count: i64,
    user_count: i64,
    channel_count: i64,
    version_count: i64,
    total_storage: String,
    message_storage: String,
    attachment_storage: String,
    names: String,
    icons: String,
    channels: String,
    users: String,
}

#[derive(Template)]
#[template(path = "channel-status.html")]
struct ChannelStatusTemplate<'a> {
    channel_name: &'a str,
    channel_id: i64,
    server_id: i64,
    server_name: &'a str,
    message_count: i64,
    user_count: i64,
    version_count: i64,
    total_storage: String,
    message_storage: String,
    attachment_storage: String,
    names: String,
    users: String,
    access_reason: &'a str,
}

#[derive(Template)]
#[template(path = "user-status.html")]
struct UserStatusTemplate<'a> {
    username: &'a str,
    user_id: i64,
    message_count: i64,
    server_count: i64,
    channel_count: i64,
    version_count: i64,
    total_storage: String,
    message_storage: String,
    attachment_storage: String,
    names: String,
    avatars: String,
    servers: String,
    channels: String,
    access_reason: &'a str,
}

#[derive(Template)]
#[template(path = "status-name.html")]
struct StatusNameTemplate<'a> {
    name: &'a str,
    first_seen: &'a str,
    last_seen: &'a str,
}

#[derive(Template)]
#[template(path = "status-icon.html")]
struct StatusIconTemplate<'a> {
    icon_url: &'a str,
    first_seen: &'a str,
    last_seen: &'a str,
}

#[derive(Template)]
#[template(path = "status-avatar.html")]
struct StatusAvatarTemplate<'a> {
    avatar_url: &'a str,
    first_seen: &'a str,
    last_seen: &'a str,
}

#[derive(Template)]
#[template(path = "status-channel.html")]
struct StatusChannelTemplate<'a> {
    channel_id: i64,
    channel_name: &'a str,
    message_count: i64,
}

#[derive(Template)]
#[template(path = "status-server.html")]
struct StatusServerTemplate<'a> {
    server_id: i64,
    server_name: &'a str,
    message_count: i64,
}

#[derive(Template)]
#[template(path = "status-user.html")]
struct StatusUserTemplate<'a> {
    user_id: i64,
    username: &'a str,
    message_count: i64,
}

#[derive(Template)]
#[template(path = "message-list.html")]
struct MessageListTemplate<'a> {
    title: &'a str,
    search_form: String,
    item_count: i64,
    items: String,
    pagination: String,
}

#[derive(Template)]
#[template(path = "message-list-item.html")]
struct MessageListItemTemplate<'a> {
    message_id: i64,
    author_id: i64,
    author: &'a str,
    server_id: i64,
    server: &'a str,
    channel_id: i64,
    channel: &'a str,
    timestamp: &'a str,
    attachment_count: i64,
    embed_count: i64,
}

#[derive(Template)]
#[template(path = "message.html")]
struct MessageTemplate<'a> {
    message_id: i64,
    author: &'a str,
    author_id: i64,
    server_id: i64,
    server: &'a str,
    channel_id: i64,
    channel: &'a str,
    timestamp: &'a str,
    version_navigation: String,
    content: &'a str,
    attachments: String,
    embeds: String,
    access_reason: &'a str,
}

#[derive(Template)]
#[template(path = "message-version-navigation.html")]
struct MessageVersionNavigationTemplate<'a> {
    message_id: i64,
    current_version: i64,
    version_count: i64,
    archived_at: &'a str,
    previous_version: String,
    next_version: String,
}

#[derive(Template)]
#[template(path = "message-version-button.html")]
struct MessageVersionButtonTemplate<'a> {
    message_id: i64,
    version: i64,
    label: &'a str,
}

#[derive(Template)]
#[template(path = "attachment.html")]
struct AttachmentTemplate<'a> {
    attachment_id: i64,
    message_version: i64,
    filename: &'a str,
    size: i64,
    content_type: Option<&'a str>,
    description: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "embed.html")]
struct EmbedTemplate<'a> {
    title: Option<&'a str>,
    description: Option<&'a str>,
    url: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "pagination.html")]
struct PaginationTemplate {
    first_pages: String,
    arbitrary_page: String,
    final_pages: String,
    current_page: i64,
    total_pages: i64,
}

#[derive(Template)]
#[template(path = "page-button.html")]
struct PageButtonTemplate<'a> {
    action: &'a str,
    search: &'a str,
    search_by: Option<&'a str>,
    page_number: i64,
    label: &'a str,
}

#[derive(Template)]
#[template(path = "current-page.html")]
struct CurrentPageTemplate {
    page_number: i64,
}

#[derive(Template)]
#[template(path = "arbitrary-page.html")]
struct ArbitraryPageTemplate<'a> {
    action: &'a str,
    search: &'a str,
    search_by: Option<&'a str>,
    total_pages: i64,
}

#[derive(Template)]
#[template(path = "message-search-form.html")]
struct MessageSearchFormTemplate<'a> {
    action: &'a str,
    search: &'a str,
    content_selected: &'a str,
    timestamp_selected: &'a str,
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorTemplate<'a> {
    message: &'a str,
}

#[derive(Deserialize)]
struct LoginQuery {
    error: Option<String>,
}

#[derive(Deserialize)]
struct OAuthQuery {
    code: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    state: String,
}

#[derive(Deserialize)]
struct OAuthToken {
    access_token: String,
}

#[derive(Deserialize)]
struct DiscordUser {
    id: String,
    username: String,
}

#[derive(Deserialize)]
struct DiscordMember {
    roles: Vec<String>,
}

#[derive(Deserialize)]
struct DiscordRole {
    id: String,
    name: String,
    permissions: String,
}

#[derive(Deserialize)]
struct DiscordChannel {
    id: String,
    permission_overwrites: Option<Vec<DiscordOverwrite>>,
}

#[derive(Deserialize)]
struct DiscordOverwrite {
    id: String,
    #[serde(rename = "type")]
    kind: u8,
    allow: String,
    deny: String,
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

pub async fn run(
    listener: tokio::net::TcpListener,
    pool: PgPool,
    bot_token: String,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    redis_url: String,
) {
    let redis = Pool::new(
        Config::from_url(&redis_url).expect("Invalid TG_BOT_REDIS_URL configuration"),
        None,
        None,
        None,
        6,
    ).expect("Failed to configure Redis pool");
    let redis_connection = redis.connect();
    redis.wait_for_connect().await.expect("Failed to connect to Redis");
    let token_rate_limiter = api::TokenRateLimiter::from_env(redis.clone())
        .expect("Invalid TG_BOT_TRUSTED_PROXY_RANGES configuration");
    let data = WebData {
        pool,
        http: reqwest::Client::new(),
        bot_token,
        client_id,
        client_secret,
        redirect_uri,
        redis: redis.clone(),
        token_rate_limiter,
        api_permission_cache: api::ApiPermissionCache::new(redis.clone()),
    };
    let archive = Router::new()
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
        .route(
            "/privacy/anonymize",
            get(anonymize_confirmation).post(anonymize_all),
        )
        .route_layer(middleware::from_fn_with_state(data.clone(), require_user));
    let sessions = SessionManagerLayer::new(RedisStore::new(redis))
        .with_secure(!data.redirect_uri.starts_with("http://"))
        .with_same_site(tower_sessions::cookie::SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(Duration::hours(12)));
    let api = api::router(data.clone());
    let app = Router::new()
        .route("/login", get(login))
        .route("/privacy", get(privacy))
        .route("/privacy-policy", get(privacy_policy))
        .route("/login/discord", get(discord_login))
        .route("/login/discord/callback", get(discord_callback))
        .route("/logout", axum::routing::post(logout))
        .route("/theme", axum::routing::post(set_theme))
        .route("/theme.css", get(theme_css))
        .route("/timezone", axum::routing::post(set_timezone))
        .route("/timezone.js", get(timezone_js))
        .merge(archive)
        .with_state(data)
        .nest("/api/v1", api)
        .layer(middleware::from_fn(theme_request))
        .layer(middleware::from_fn(timezone_request))
        .layer(sessions);

    info!("Web server listening on {}", listener.local_addr().unwrap());
    if let Err(error) = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    {
        error!("Web server error: {}", error);
    }
    redis_connection.abort();
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
        self.render_with_fields(action, search, None)
    }

    fn render_messages(&self, action: &str, search: &str, search_by: &str) -> String {
        self.render_with_fields(action, search, Some(search_by))
    }

    fn render_with_fields(&self, action: &str, search: &str, search_by: Option<&str>) -> String {
        let mut first_pages = String::new();
        if self.current_page > 1 {
            first_pages.push_str(&page_button(
                action,
                search,
                search_by,
                self.current_page - 1,
                "<",
            ));
        }
        for page_number in 1..=self.total_pages.min(SHOWN_PAGES) {
            first_pages.push_str(&self.number(action, search, search_by, page_number));
        }

        let mut arbitrary_page = String::new();
        let mut final_pages = String::new();
        if self.total_pages > SHOWN_PAGES {
            arbitrary_page = render_template(&ArbitraryPageTemplate {
                action,
                search,
                search_by,
                total_pages: self.total_pages,
            });
            for page_number in (self.total_pages - 2).max(SHOWN_PAGES + 1)..=self.total_pages {
                final_pages.push_str(&self.number(action, search, search_by, page_number));
            }
        }
        if self.current_page < self.total_pages {
            final_pages.push_str(&page_button(
                action,
                search,
                search_by,
                self.current_page + 1,
                ">",
            ));
        }

        render_template(&PaginationTemplate {
            first_pages,
            arbitrary_page,
            final_pages,
            current_page: self.current_page,
            total_pages: self.total_pages,
        })
    }

    fn number(
        &self,
        action: &str,
        search: &str,
        search_by: Option<&str>,
        page_number: i64,
    ) -> String {
        if page_number == self.current_page {
            render_template(&CurrentPageTemplate { page_number })
        } else {
            page_button(
                action,
                search,
                search_by,
                page_number,
                &page_number.to_string(),
            )
        }
    }
}

fn page_button(
    action: &str,
    search: &str,
    search_by: Option<&str>,
    page_number: i64,
    label: &str,
) -> String {
    render_template(&PageButtonTemplate {
        action,
        search,
        search_by,
        page_number,
        label,
    })
}

fn search_form(action: &str, search: &str, label: &str) -> String {
    render_template(&SearchFormTemplate {
        action,
        label,
        search,
    })
}

fn render_status_names(names: Vec<(String, String, String)>) -> String {
    let mut rendered_names = String::new();
    for (name, first_seen, last_seen) in names {
        let first_seen = localize_timestamp(&first_seen);
        let last_seen = localize_timestamp(&last_seen);
        rendered_names.push_str(&render_template(&StatusNameTemplate {
            name: &name,
            first_seen: &first_seen,
            last_seen: &last_seen,
        }));
    }
    rendered_names
}

fn render_status_icons(icons: Vec<(String, String, String)>) -> String {
    let mut rendered_icons = String::new();
    for (icon_url, first_seen, last_seen) in icons {
        let first_seen = localize_timestamp(&first_seen);
        let last_seen = localize_timestamp(&last_seen);
        rendered_icons.push_str(&render_template(&StatusIconTemplate {
            icon_url: &icon_url,
            first_seen: &first_seen,
            last_seen: &last_seen,
        }));
    }
    rendered_icons
}

fn render_status_avatars(avatars: Vec<(String, String, String)>) -> String {
    let mut rendered_avatars = String::new();
    for (avatar_url, first_seen, last_seen) in avatars {
        let first_seen = localize_timestamp(&first_seen);
        let last_seen = localize_timestamp(&last_seen);
        rendered_avatars.push_str(&render_template(&StatusAvatarTemplate {
            avatar_url: &avatar_url,
            first_seen: &first_seen,
            last_seen: &last_seen,
        }));
    }
    rendered_avatars
}

fn render_status_channels(channels: Vec<(i64, String, i64)>) -> String {
    let mut rendered_channels = String::new();
    for (channel_id, channel_name, message_count) in channels {
        rendered_channels.push_str(&render_template(&StatusChannelTemplate {
            channel_id,
            channel_name: &channel_name,
            message_count,
        }));
    }
    rendered_channels
}

fn render_status_servers(servers: Vec<(i64, String, i64)>) -> String {
    let mut rendered_servers = String::new();
    for (server_id, server_name, message_count) in servers {
        rendered_servers.push_str(&render_template(&StatusServerTemplate {
            server_id,
            server_name: &server_name,
            message_count,
        }));
    }
    rendered_servers
}

fn render_status_users(users: Vec<(i64, String, i64)>) -> String {
    let mut rendered_users = String::new();
    for (user_id, username, message_count) in users {
        rendered_users.push_str(&render_template(&StatusUserTemplate {
            user_id,
            username: &username,
            message_count,
        }));
    }
    rendered_users
}

fn message_search_form(action: &str, search: &str, search_by: &str) -> String {
    let (content_selected, timestamp_selected) = if search_by == "timestamp" {
        ("", " selected")
    } else {
        (" selected", "")
    };
    render_template(&MessageSearchFormTemplate {
        action,
        search,
        content_selected,
        timestamp_selected,
    })
}

fn message_version_navigation(
    message_id: i64,
    current_version: i64,
    version_count: i64,
    archived_at: &str,
) -> String {
    let archived_at = localize_timestamp(archived_at);
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
    render_template(&MessageVersionNavigationTemplate {
        message_id,
        current_version,
        version_count,
        archived_at: &archived_at,
        previous_version,
        next_version,
    })
}

fn message_version_button(message_id: i64, version: i64, label: &str) -> String {
    render_template(&MessageVersionButtonTemplate {
        message_id,
        version,
        label,
    })
}

fn page(title: &str, body: &str) -> String {
    render_page(title, body, true)
}

fn render_page(title: &str, body: &str, logged_in: bool) -> String {
    let theme = ACTIVE_THEME
        .try_with(|theme| theme.as_str())
        .unwrap_or("white");
    let detect_timezone = ACTIVE_TIMEZONE
        .try_with(|context| context.detect)
        .unwrap_or(false);
    render_template(&PageTemplate {
        title,
        body,
        theme,
        logged_in,
        detect_timezone,
    })
}

fn localize_timestamp(value: &str) -> String {
    let timezone = ACTIVE_TIMEZONE
        .try_with(|context| context.timezone)
        .unwrap_or(chrono_tz::UTC);
    chrono::DateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f%#z")
        .map(|timestamp| {
            timestamp
                .with_timezone(&timezone)
                .format("%Y-%m-%d %H:%M:%S %Z")
                .to_string()
        })
        .unwrap_or_else(|_| value.to_string())
}

fn render_template(template: &impl Template) -> String {
    template.render().unwrap()
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

fn database_error(error: sqlx::Error) -> (StatusCode, Html<String>) {
    error!("Database error while rendering web page: {}", error);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(page(
            "Error",
            &render_template(&ErrorTemplate {
                message: "The archive could not be loaded.",
            }),
        )),
    )
}

fn not_found(message: &str) -> (StatusCode, Html<String>) {
    (
        StatusCode::NOT_FOUND,
        Html(page(
            "Not found",
            &render_template(&ErrorTemplate { message }),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_from_the_database() {
        let html = render_template(&StatusNameTemplate {
            name: "<script>'\"&</script>",
            first_seen: "<old>",
            last_seen: "<new>",
        });

        assert!(html.contains("&#60;script&#62;&#39;&#34;&#38;&#60;/script&#62;"));
        assert!(html.contains("&#60;old&#62;"));
        assert!(html.contains("&#60;new&#62;"));
    }

    #[test]
    fn login_page_renders_and_escapes_query_error() {
        let html = render_template(&LoginTemplate {
            error: Some("Login failed: <try again>"),
        });

        assert!(html.contains("Login failed: &#60;try again&#62;"));
    }

    #[test]
    fn renders_short_pagination_without_arbitrary_page_form() {
        let pagination = Pagination::new(2, ITEMS_PER_PAGE * 3);
        let html = pagination.render("/messages", "test");

        assert!(html.contains("Page 2 of 3"));
        assert!(!html.contains("type=\"number\""));
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
    }

    #[test]
    fn message_pagination_keeps_the_search_type() {
        let pagination = Pagination::new(2, ITEMS_PER_PAGE * 3);
        let html = pagination.render_messages("/servers/123", "2026-08", "timestamp");

        assert!(html.contains("action=\"/servers/123\""));
        assert!(html.contains("name=\"search\" value=\"2026-08\""));
        assert!(html.contains("name=\"search_by\" value=\"timestamp\""));
    }

    #[test]
    fn message_search_form_selects_timestamp_and_escapes_the_search() {
        let html = message_search_form("/users/123", "<date>", "timestamp");

        assert!(html.contains("action=\"/users/123\""));
        assert!(html.contains("value=\"&#60;date&#62;\""));
        assert!(html.contains("value=\"timestamp\" selected"));
        assert!(!html.contains("value=\"content\" selected"));
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

        assert!(html.contains("&#60;old name&#62;"));
        assert!(html.contains("first seen 2026-01-01"));
    }

    #[test]
    fn channel_permissions_apply_role_and_member_overwrites() {
        let roles = HashMap::from([(1, 0), (2, 1 << 10)]);
        let overwrites = vec![
            DiscordOverwrite {
                id: "2".into(),
                kind: 0,
                allow: "0".into(),
                deny: (1_u64 << 10).to_string(),
            },
            DiscordOverwrite {
                id: "3".into(),
                kind: 1,
                allow: (1_u64 << 10).to_string(),
                deny: "0".into(),
            },
        ];

        assert!(can_view_channel(1, 3, &[2], &roles, &overwrites));
        assert!(!can_view_channel(1, 4, &[2], &roles, &overwrites));
    }

    #[test]
    fn administrator_bypasses_channel_overwrites() {
        let roles = HashMap::from([(1, 0), (2, 1 << 3)]);
        let overwrites = vec![DiscordOverwrite {
            id: "2".into(),
            kind: 0,
            allow: "0".into(),
            deny: u64::MAX.to_string(),
        }];

        assert!(can_view_channel(1, 3, &[2], &roles, &overwrites));
    }

    #[test]
    fn explains_role_based_channel_access() {
        let roles = HashMap::from([(1, 0), (2, 1 << 10)]);
        let role_names = HashMap::from([(1, "@everyone".into()), (2, "Archivist".into())]);

        assert_eq!(
            view_channel_reason(1, 3, &[2], &roles, &role_names, &[], "TeenGovernment"),
            Some("you have the Archivist role in TeenGovernment".into())
        );
    }

    #[test]
    fn explains_server_membership_channel_access() {
        let roles = HashMap::from([(1, 1 << 10)]);
        let role_names = HashMap::from([(1, "@everyone".into())]);

        assert_eq!(
            view_channel_reason(1, 3, &[], &roles, &role_names, &[], "TeenGovernment"),
            Some("you are a member of TeenGovernment".into())
        );
    }

    #[test]
    fn validates_cookie_themes() {
        assert_eq!(Theme::from_cookie(Some("white")).as_str(), "white");
        assert_eq!(Theme::from_cookie(Some("black")).as_str(), "black");
        assert_eq!(Theme::from_cookie(Some("oled")).as_str(), "oled");
        assert_eq!(Theme::from_cookie(Some("unknown")).as_str(), "white");
    }

    #[tokio::test]
    async fn localizes_timestamps_and_only_bootstraps_timezone_when_needed() {
        ACTIVE_TIMEZONE
            .scope(
                TimezoneContext {
                    timezone: chrono_tz::Europe::Paris,
                    detect: true,
                },
                async {
                    assert_eq!(
                        localize_timestamp("2026-08-31 12:00:00+00"),
                        "2026-08-31 14:00:00 CEST"
                    );
                    assert!(
                        render_page("Archive", "Content", true)
                            .contains("<script src=\"/timezone.js\" defer></script>")
                    );
                },
            )
            .await;

        ACTIVE_TIMEZONE
            .scope(
                TimezoneContext {
                    timezone: chrono_tz::Europe::Paris,
                    detect: false,
                },
                async {
                    assert!(!render_page("Archive", "Content", true).contains("/timezone.js"));
                },
            )
            .await;
    }

    #[test]
    fn refreshes_expired_or_legacy_channel_access() {
        let mut user = WebUser {
            id: 1,
            username: "user".into(),
            channel_ids: vec![10],
            channel_access: Vec::new(),
            channel_access_refreshed_at: 1_000,
        };

        assert!(!channel_access_needs_refresh_at(&user, "/channels", 1_299));
        assert!(channel_access_needs_refresh_at(&user, "/channels", 1_300));
        user.channel_access_refreshed_at = 0;
        assert!(channel_access_needs_refresh_at(&user, "/channels", 1_000));
    }

    #[test]
    fn refreshes_when_a_directly_requested_channel_is_not_cached() {
        let user = WebUser {
            id: 1,
            username: "user".into(),
            channel_ids: vec![10],
            channel_access: Vec::new(),
            channel_access_refreshed_at: 1_000,
        };

        assert!(!channel_access_needs_refresh_at(
            &user,
            "/channels/10/messages",
            1_001
        ));
        assert!(channel_access_needs_refresh_at(
            &user,
            "/channels/20/messages",
            1_001
        ));
    }

    #[test]
    fn shared_page_contains_the_footer_and_theme_switcher() {
        let html = render_page("Archive", "Content", true);

        assert!(html.contains("class=\"theme-white\""));
        assert!(!html.contains("If you cannot see any messages"));
        assert!(include_str!("../../static/index.html").contains("If you cannot see any messages"));
        assert!(html.contains("github.com/eliaswen/tg-archive"));
        assert!(html.contains("Copyright © 2026 Ewi and contributors"));
        assert!(html.contains("licensed under GPL-3.0"));
        assert!(html.contains("href=\"/privacy\""));
        assert!(html.contains("action=\"/theme\""));
    }

    #[test]
    fn privacy_page_hides_anonymization_when_logged_out() {
        let body = render_template(&PrivacyTemplate { logged_in: false });
        let html = render_page("Privacy", &body, false);

        assert!(html.contains("<title>Privacy - TG Archive</title>"));
        assert!(html.contains("<main class=\"page-content\">\n\n    </main>"));
        assert!(!html.contains("Anonymize all"));
    }

    #[test]
    fn privacy_page_offers_anonymization_when_logged_in() {
        let body = render_template(&PrivacyTemplate { logged_in: true });

        assert!(body.contains("action=\"/privacy/anonymize\""));
        assert!(body.contains("Anonymize all"));
    }

    #[test]
    fn anonymization_requires_a_separate_confirmation_page() {
        let body = render_template(&AnonymizeConfirmationTemplate);

        assert!(body.contains("method=\"post\""));
        assert!(body.contains("action=\"/privacy/anonymize\""));
        assert!(body.contains("Confirm anonymization"));
    }
}
