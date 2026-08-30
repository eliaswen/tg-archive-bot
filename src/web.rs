use askama::Template;
use axum::{
    Extension, Form, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use rand::{Rng, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use tower_sessions::{Expiry, MemoryStore, Session, SessionManagerLayer, cookie::time::Duration};
use tracing::{error, info};

tokio::task_local! {
    static ACTIVE_THEME: Theme;
}

const ITEMS_PER_PAGE: i64 = 100;
const SHOWN_PAGES: i64 = 10;

#[derive(Clone)]
struct WebData {
    pool: PgPool,
    http: reqwest::Client,
    bot_token: String,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct WebUser {
    id: i64,
    username: String,
    channel_ids: Vec<i64>,
    channel_access: Vec<ChannelAccess>,
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
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate;

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
struct OAuthQuery {
    code: String,
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

pub struct ArchiveStats {
    pub messages: i64,
    pub users: i64,
    pub servers: i64,
    pub channels: i64,
    pub total_storage: i64,
    pub message_storage: i64,
    pub attachment_storage: i64,
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
) {
    let data = WebData {
        pool,
        http: reqwest::Client::new(),
        bot_token,
        client_id,
        client_secret,
        redirect_uri,
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
        .route_layer(middleware::from_fn_with_state(data.clone(), require_user));
    let sessions = SessionManagerLayer::new(MemoryStore::default())
        .with_secure(!data.redirect_uri.starts_with("http://"))
        .with_same_site(tower_sessions::cookie::SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(Duration::hours(12)));
    let app = Router::new()
        .route("/login", get(login))
        .route("/login/discord", get(discord_login))
        .route("/login/discord/callback", get(discord_callback))
        .route("/logout", axum::routing::post(logout))
        .route("/theme", axum::routing::post(set_theme))
        .route("/theme.css", get(theme_css))
        .merge(archive)
        .with_state(data)
        .layer(middleware::from_fn(theme_request))
        .layer(sessions);

    info!("Web server listening on {}", listener.local_addr().unwrap());
    if let Err(error) = axum::serve(listener, app).await {
        error!("Web server error: {}", error);
    }
}

async fn theme_request(jar: CookieJar, request: Request, next: Next) -> Response {
    let theme = Theme::from_cookie(jar.get("theme").map(Cookie::value));
    ACTIVE_THEME.scope(theme, next.run(request)).await
}

async fn require_user(
    State(data): State<WebData>,
    session: Session,
    mut request: Request,
    next: Next,
) -> Response {
    match session.get::<WebUser>("user").await {
        Ok(Some(user)) => {
            if !request_is_allowed(&data.pool, &user, request.uri().path()).await {
                return StatusCode::NOT_FOUND.into_response();
            }
            request.extensions_mut().insert(user);
            next.run(request).await
        }
        Ok(None) => Redirect::to("/login").into_response(),
        Err(error) => {
            error!("Session error: {}", error);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn request_is_allowed(pool: &PgPool, user: &WebUser, path: &str) -> bool {
    let parts = path.trim_matches('/').split('/').collect::<Vec<_>>();
    let channel_id = match parts.as_slice() {
        ["channels", id, ..] => id.parse::<i64>().ok(),
        ["messages", id] => match id.parse::<i64>() {
            Ok(id) => sqlx::query_scalar("SELECT channel_id FROM messages WHERE message_id = $1")
                .bind(id)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten(),
            Err(_) => None,
        },
        ["attachments", id] => match id.parse::<i64>() {
            Ok(id) => sqlx::query_scalar(
                "SELECT m.channel_id FROM attachments a JOIN messages m ON m.message_id = a.message_id WHERE a.attachment_id = $1",
            )
            .bind(id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten(),
            Err(_) => None,
        },
        ["servers", id, ..] => return match id.parse::<i64>() {
            Ok(id) => sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM channels WHERE guild_id = $1 AND channel_id = ANY($2))",
            )
            .bind(id)
            .bind(&user.channel_ids)
            .fetch_one(pool)
            .await
            .unwrap_or(false),
            Err(_) => false,
        },
        ["users", id, ..] => return match id.parse::<i64>() {
            Ok(id) => sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE author_id = $1 AND channel_id = ANY($2))",
            )
            .bind(id)
            .bind(&user.channel_ids)
            .fetch_one(pool)
            .await
            .unwrap_or(false),
            Err(_) => false,
        },
        _ => return true,
    };
    channel_id.is_some_and(|channel_id| user.channel_ids.contains(&channel_id))
}

async fn login(session: Session) -> Response {
    match session.get::<WebUser>("user").await {
        Ok(Some(_)) => Redirect::to("/").into_response(),
        Ok(None) => Html(render_page(
            "Log in",
            &render_template(&LoginTemplate),
            false,
        ))
        .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn set_theme(
    State(data): State<WebData>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(form): Form<ThemeForm>,
) -> Response {
    let theme = Theme::from_cookie(Some(&form.theme));
    let cookie = Cookie::build(("theme", theme.as_str()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(!data.redirect_uri.starts_with("http://"))
        .max_age(Duration::days(365))
        .build();
    let return_to = headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| reqwest::Url::parse(value).ok())
        .map(|url| {
            let mut path = url.path().to_string();
            if let Some(query) = url.query() {
                path.push('?');
                path.push_str(query);
            }
            path
        })
        .unwrap_or_else(|| "/".into());
    (jar.add(cookie), Redirect::to(&return_to)).into_response()
}

async fn theme_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/theme.css"),
    )
}

async fn discord_login(State(data): State<WebData>, session: Session) -> Response {
    let state: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect();
    if session.insert("oauth_state", &state).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let url = format!(
        "https://discord.com/oauth2/authorize?response_type=code&client_id={}&scope=identify&state={}&redirect_uri={}",
        data.client_id,
        state,
        urlencoding::encode(&data.redirect_uri),
    );
    Redirect::to(&url).into_response()
}

async fn discord_callback(
    State(data): State<WebData>,
    session: Session,
    Query(query): Query<OAuthQuery>,
) -> Response {
    let state = session.remove::<String>("oauth_state").await;
    if !matches!(state, Ok(Some(state)) if state == query.state) {
        return (StatusCode::BAD_REQUEST, "Invalid OAuth state").into_response();
    }
    let token = data
        .http
        .post("https://discord.com/api/v10/oauth2/token")
        .form(&[
            ("client_id", data.client_id.as_str()),
            ("client_secret", data.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", query.code.as_str()),
            ("redirect_uri", data.redirect_uri.as_str()),
        ])
        .send()
        .await;
    let Ok(token) = token else {
        return oauth_error("Discord token request failed");
    };
    let Ok(token) = token.error_for_status() else {
        return oauth_error("Discord rejected the login request");
    };
    let Ok(token) = token.json::<OAuthToken>().await else {
        return oauth_error("Discord returned an invalid token response");
    };
    let user = data
        .http
        .get("https://discord.com/api/v10/users/@me")
        .bearer_auth(&token.access_token)
        .send()
        .await;
    let Ok(user) = user else {
        return oauth_error("Discord user request failed");
    };
    let Ok(user) = user.error_for_status() else {
        return oauth_error("Discord rejected the user request");
    };
    let Ok(user) = user.json::<DiscordUser>().await else {
        return oauth_error("Discord returned an invalid user response");
    };
    let Ok(user_id) = user.id.parse::<i64>() else {
        return oauth_error("Discord returned an invalid user ID");
    };
    let channel_access = match accessible_channels(&data, user_id).await {
        Ok(channel_access) => channel_access,
        Err(error) => {
            error!("Discord permission check failed: {}", error);
            return oauth_error("Could not check your current Discord permissions");
        }
    };
    let channel_ids = channel_access
        .iter()
        .map(|access| access.channel_id)
        .collect();
    let user = WebUser {
        id: user_id,
        username: user.username,
        channel_ids,
        channel_access,
    };
    if session.cycle_id().await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if session.insert("user", user).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    Redirect::to("/").into_response()
}

async fn logout(session: Session) -> Redirect {
    let _ = session.delete().await;
    Redirect::to("/login")
}

fn oauth_error(message: &str) -> Response {
    (StatusCode::BAD_GATEWAY, message.to_string()).into_response()
}

async fn accessible_channels(
    data: &WebData,
    user_id: i64,
) -> Result<Vec<ChannelAccess>, Box<dyn std::error::Error + Send + Sync>> {
    let guilds = sqlx::query_as::<_, (i64, String)>(
        "SELECT guild_id, guild_name FROM guilds ORDER BY guild_id;",
    )
    .fetch_all(&data.pool)
    .await?;
    let bot = discord_get::<DiscordUser>(data, "/users/@me").await?;
    let bot_id = bot.id.parse::<i64>()?;
    let archived_channels = sqlx::query_scalar::<_, i64>("SELECT channel_id FROM channels;")
        .fetch_all(&data.pool)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    let mut visible = Vec::new();

    for (guild_id, guild_name) in guilds {
        let user = discord_get_optional::<DiscordMember>(
            data,
            &format!("/guilds/{}/members/{}", guild_id, user_id),
        )
        .await?;
        let Some(user) = user else {
            continue;
        };
        let bot = discord_get_optional::<DiscordMember>(
            data,
            &format!("/guilds/{}/members/{}", guild_id, bot_id),
        )
        .await?;
        let Some(bot) = bot else {
            continue;
        };
        let roles =
            discord_get::<Vec<DiscordRole>>(data, &format!("/guilds/{}/roles", guild_id)).await?;
        let channels =
            discord_get::<Vec<DiscordChannel>>(data, &format!("/guilds/{}/channels", guild_id))
                .await?;
        let role_names = roles
            .iter()
            .map(|role| Ok((role.id.parse::<i64>()?, role.name.clone())))
            .collect::<Result<HashMap<_, _>, Box<dyn std::error::Error + Send + Sync>>>()?;
        let role_permissions = roles
            .iter()
            .map(|role| Ok((role.id.parse::<i64>()?, role.permissions.parse::<u64>()?)))
            .collect::<Result<HashMap<_, _>, Box<dyn std::error::Error + Send + Sync>>>()?;
        let user_roles = user
            .roles
            .iter()
            .map(|role| role.parse::<i64>())
            .collect::<Result<Vec<_>, _>>()?;
        let bot_roles = bot
            .roles
            .iter()
            .map(|role| role.parse::<i64>())
            .collect::<Result<Vec<_>, _>>()?;

        for channel in channels {
            let channel_id = channel.id.parse::<i64>()?;
            if !archived_channels.contains(&channel_id) {
                continue;
            }
            let overwrites = channel.permission_overwrites.as_deref().unwrap_or_default();
            let reason = view_channel_reason(
                guild_id,
                user_id,
                &user_roles,
                &role_permissions,
                &role_names,
                overwrites,
                &guild_name,
            );
            if let Some(reason) = reason
                && can_view_channel(guild_id, bot_id, &bot_roles, &role_permissions, overwrites)
            {
                visible.push(ChannelAccess { channel_id, reason });
            }
        }
    }

    visible.sort_unstable_by_key(|access| access.channel_id);
    Ok(visible)
}

async fn discord_get<T: serde::de::DeserializeOwned>(
    data: &WebData,
    path: &str,
) -> Result<T, Box<dyn std::error::Error + Send + Sync>> {
    let response = data
        .http
        .get(format!("https://discord.com/api/v10{}", path))
        .header("Authorization", format!("Bot {}", data.bot_token))
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

async fn discord_get_optional<T: serde::de::DeserializeOwned>(
    data: &WebData,
    path: &str,
) -> Result<Option<T>, Box<dyn std::error::Error + Send + Sync>> {
    let response = data
        .http
        .get(format!("https://discord.com/api/v10{}", path))
        .header("Authorization", format!("Bot {}", data.bot_token))
        .send()
        .await?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(Some(response.error_for_status()?.json().await?))
}

fn can_view_channel(
    guild_id: i64,
    user_id: i64,
    member_roles: &[i64],
    roles: &HashMap<i64, u64>,
    overwrites: &[DiscordOverwrite],
) -> bool {
    const ADMINISTRATOR: u64 = 1 << 3;
    const VIEW_CHANNEL: u64 = 1 << 10;
    let mut permissions = roles.get(&guild_id).copied().unwrap_or_default();
    for role_id in member_roles {
        permissions |= roles.get(role_id).copied().unwrap_or_default();
    }
    if permissions & ADMINISTRATOR != 0 {
        return true;
    }
    apply_overwrite(&mut permissions, overwrites, guild_id, 0);
    let mut allow = 0;
    let mut deny = 0;
    for overwrite in overwrites {
        let Ok(role_id) = overwrite.id.parse::<i64>() else {
            continue;
        };
        if overwrite.kind == 0 && member_roles.contains(&role_id) {
            allow |= overwrite.allow.parse::<u64>().unwrap_or_default();
            deny |= overwrite.deny.parse::<u64>().unwrap_or_default();
        }
    }
    permissions &= !deny;
    permissions |= allow;
    apply_overwrite(&mut permissions, overwrites, user_id, 1);
    permissions & VIEW_CHANNEL != 0
}

fn view_channel_reason(
    guild_id: i64,
    user_id: i64,
    member_roles: &[i64],
    roles: &HashMap<i64, u64>,
    role_names: &HashMap<i64, String>,
    overwrites: &[DiscordOverwrite],
    guild_name: &str,
) -> Option<String> {
    const ADMINISTRATOR: u64 = 1 << 3;
    const VIEW_CHANNEL: u64 = 1 << 10;
    if !can_view_channel(guild_id, user_id, member_roles, roles, overwrites) {
        return None;
    }
    for role_id in member_roles {
        if roles.get(role_id).copied().unwrap_or_default() & ADMINISTRATOR != 0 {
            let role_name = role_names
                .get(role_id)
                .map(String::as_str)
                .unwrap_or("unknown");
            return Some(format!(
                "you have the {} administrator role in {}",
                role_name, guild_name
            ));
        }
    }
    let member_allows_view = overwrites.iter().any(|overwrite| {
        overwrite.kind == 1
            && overwrite.id.parse::<i64>() == Ok(user_id)
            && overwrite.allow.parse::<u64>().unwrap_or_default() & VIEW_CHANNEL != 0
    });
    if member_allows_view {
        return Some(format!(
            "you have a channel-specific permission in {}",
            guild_name
        ));
    }
    for role_id in member_roles {
        let role_allows_view = overwrites.iter().any(|overwrite| {
            overwrite.kind == 0
                && overwrite.id.parse::<i64>() == Ok(*role_id)
                && overwrite.allow.parse::<u64>().unwrap_or_default() & VIEW_CHANNEL != 0
        });
        if role_allows_view {
            let role_name = role_names
                .get(role_id)
                .map(String::as_str)
                .unwrap_or("unknown");
            return Some(format!("you have the {} role in {}", role_name, guild_name));
        }
    }
    let everyone_allows_view = overwrites.iter().any(|overwrite| {
        overwrite.kind == 0
            && overwrite.id.parse::<i64>() == Ok(guild_id)
            && overwrite.allow.parse::<u64>().unwrap_or_default() & VIEW_CHANNEL != 0
    });
    if everyone_allows_view {
        return Some(format!("you are a member of {}", guild_name));
    }
    for role_id in member_roles {
        if roles.get(role_id).copied().unwrap_or_default() & VIEW_CHANNEL != 0 {
            let role_name = role_names
                .get(role_id)
                .map(String::as_str)
                .unwrap_or("unknown");
            return Some(format!("you have the {} role in {}", role_name, guild_name));
        }
    }
    Some(format!("you are a member of {}", guild_name))
}

fn apply_overwrite(permissions: &mut u64, overwrites: &[DiscordOverwrite], id: i64, kind: u8) {
    let overwrite = overwrites
        .iter()
        .find(|overwrite| overwrite.kind == kind && overwrite.id.parse::<i64>() == Ok(id));
    if let Some(overwrite) = overwrite {
        *permissions &= !overwrite.deny.parse::<u64>().unwrap_or_default();
        *permissions |= overwrite.allow.parse::<u64>().unwrap_or_default();
    }
}

async fn index(State(data): State<WebData>) -> WebResult {
    let stats = archive_stats(&data.pool).await.map_err(database_error)?;
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

pub async fn archive_stats(pool: &PgPool) -> Result<ArchiveStats, sqlx::Error> {
    let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64)>(
        "SELECT
             (SELECT COUNT(*) FROM messages),
             (SELECT COUNT(*) FROM discord_users),
             (SELECT COUNT(*) FROM guilds),
             (SELECT COUNT(*) FROM channels),
             (SELECT COALESCE(SUM(OCTET_LENGTH(content)), 0)
              FROM message_versions)
                 + (SELECT COALESCE(SUM(OCTET_LENGTH(data)), 0)
                    FROM attachments),
             (SELECT COALESCE(SUM(OCTET_LENGTH(content)), 0)
              FROM message_versions),
             (SELECT COALESCE(SUM(OCTET_LENGTH(data)), 0)
              FROM attachments);",
    )
    .fetch_one(pool)
    .await?;
    Ok(ArchiveStats {
        messages: stats.0,
        users: stats.1,
        servers: stats.2,
        channels: stats.3,
        total_storage: stats.4,
        message_storage: stats.5,
        attachment_storage: stats.6,
    })
}

async fn servers(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_servers(&data.pool, "", query.page.unwrap_or(1), &user.channel_ids).await
}

async fn search_servers(
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

async fn server(
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

async fn server_messages(
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
    )
    .await
}

async fn search_server_messages(
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
    )
    .await
}

async fn render_server_messages(
    pool: &PgPool,
    server_id: i64,
    search: &str,
    search_by: &str,
    requested_page: i64,
    channel_ids: &[i64],
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
    )
    .await
}

async fn channel_messages(
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
    )
    .await
}

async fn search_channel_messages(
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
    )
    .await
}

async fn render_channel_messages(
    pool: &PgPool,
    channel_id: i64,
    search: &str,
    search_by: &str,
    requested_page: i64,
    channel_ids: &[i64],
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
    )
    .await
}

async fn render_servers(
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

async fn channels(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_channels(&data.pool, "", query.page.unwrap_or(1), &user.channel_ids).await
}

async fn search_channels(
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

async fn channel(
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

async fn render_channels(
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

async fn users(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Query(query): Query<PageQuery>,
) -> WebResult {
    render_users(&data.pool, "", query.page.unwrap_or(1), &user.channel_ids).await
}

async fn search_users(
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

async fn user(
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

async fn user_messages(
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
    )
    .await
}

async fn search_user_messages(
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
    )
    .await
}

async fn render_user_messages(
    pool: &PgPool,
    user_id: i64,
    search: &str,
    search_by: &str,
    requested_page: i64,
    channel_ids: &[i64],
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
    )
    .await
}

async fn render_users(
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

async fn messages(
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
    )
    .await
}

async fn search_messages(
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
    )
    .await
}

async fn render_messages(
    pool: &PgPool,
    search: &str,
    search_by: &str,
    requested_page: i64,
    scope: MessageScope,
    channel_ids: &[i64],
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
           AND m.channel_id = ANY($6);",
    )
    .bind(&search_pattern)
    .bind(search_by)
    .bind(scope.server_id())
    .bind(scope.user_id())
    .bind(scope.channel_id())
    .bind(channel_ids)
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
           AND m.channel_id = ANY($6)
         ORDER BY m.timestamp DESC, m.message_id DESC
         LIMIT $7 OFFSET $8;",
        )
        .bind(&search_pattern)
        .bind(search_by)
        .bind(scope.server_id())
        .bind(scope.user_id())
        .bind(scope.channel_id())
        .bind(channel_ids)
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

async fn message(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
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
        version_navigation: message_version_navigation(
            message_id,
            message_version,
            version_count,
            &archived_at,
        ),
        content: content.as_deref().unwrap_or(""),
        attachments: rendered_attachments,
        embeds: rendered_embeds,
        access_reason: user.access_reason(channel_id),
    });
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
        archived_at,
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
    render_template(&PageTemplate {
        title,
        body,
        theme,
        logged_in,
    })
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

pub fn format_bytes(bytes: i64) -> String {
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

    #[test]
    fn shared_page_contains_the_footer_and_theme_switcher() {
        let html = render_page("Archive", "Content", true);

        assert!(html.contains("class=\"theme-white\""));
        assert!(!html.contains("If you cannot see any messages"));
        assert!(include_str!("../static/index.html").contains("If you cannot see any messages"));
        assert!(html.contains("git.ewenlau.net/ewenlau/tg-archive-bot"));
        assert!(html.contains("Copyright © 2026 Ewi and contributors"));
        assert!(html.contains("licensed under GPL-3.0"));
        assert!(html.contains("action=\"/theme\""));
    }
}
