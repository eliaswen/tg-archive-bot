use super::{
    CHANNEL_ACCESS_TTL_SECONDS, ChannelAccess, WebData, WebUser, accessible_channels, safe_filename,
};
use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rand::{RngExt, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tower_sessions::Session;

const RESULTS_PER_PAGE: i64 = 100;
const TOKEN_RATE_LIMIT: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Default)]
pub(super) struct ApiPermissionCache {
    users: Arc<Mutex<HashMap<i64, (Instant, Vec<ChannelAccess>)>>>,
}

impl ApiPermissionCache {
    fn get_at(&self, discord_id: i64, now: Instant) -> Option<Vec<ChannelAccess>> {
        let mut users = self.users.lock().unwrap_or_else(|error| error.into_inner());
        users.retain(|_, (refreshed_at, _)| {
            now.saturating_duration_since(*refreshed_at).as_secs() < CHANNEL_ACCESS_TTL_SECONDS
        });
        users
            .get(&discord_id)
            .map(|(_, channel_access)| channel_access.clone())
    }

    fn get(&self, discord_id: i64) -> Option<Vec<ChannelAccess>> {
        self.get_at(discord_id, Instant::now())
    }

    fn insert_at(&self, discord_id: i64, channel_access: Vec<ChannelAccess>, now: Instant) {
        self.users
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(discord_id, (now, channel_access));
    }

    fn insert(&self, discord_id: i64, channel_access: Vec<ChannelAccess>) {
        self.insert_at(discord_id, channel_access, Instant::now());
    }
}

async fn cached_accessible_channels(
    data: &WebData,
    discord_id: i64,
) -> Result<Vec<ChannelAccess>, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(channel_access) = data.api_permission_cache.get(discord_id) {
        return Ok(channel_access);
    }

    let channel_access = accessible_channels(data, discord_id).await?;
    data.api_permission_cache
        .insert(discord_id, channel_access.clone());
    Ok(channel_access)
}

#[derive(Clone)]
pub(super) struct TokenRateLimiter {
    attempts: Arc<Mutex<HashMap<IpAddr, Instant>>>,
    trusted_proxies: Arc<Vec<ipnet::IpNet>>,
}

impl TokenRateLimiter {
    pub(super) fn from_env() -> Result<Self, String> {
        let trusted_proxies = env::var("TG_BOT_TRUSTED_PROXY_RANGES")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|range| !range.is_empty())
            .map(|range| {
                range
                    .parse::<ipnet::IpNet>()
                    .or_else(|_| range.parse::<IpAddr>().map(ipnet::IpNet::from))
                    .map_err(|_| format!("invalid trusted proxy IP range: {range}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
            trusted_proxies: Arc::new(trusted_proxies),
        })
    }

    fn client_ip(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        if self
            .trusted_proxies
            .iter()
            .any(|range| range.contains(&peer))
        {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(peer)
        } else {
            peer
        }
    }

    fn check(&self, ip: IpAddr) -> Result<(), u64> {
        let now = Instant::now();
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        attempts.retain(|_, attempt| now.duration_since(*attempt) < TOKEN_RATE_LIMIT);
        if let Some(attempt) = attempts.get(&ip) {
            let remaining = TOKEN_RATE_LIMIT.saturating_sub(now.duration_since(*attempt));
            return Err(remaining.as_secs().max(1));
        }
        attempts.insert(ip, now);
        Ok(())
    }
}

#[derive(Clone)]
struct ApiUser {
    discord_id: i64,
    channel_ids: Vec<i64>,
}
#[derive(Serialize)]
struct ErrorResponse {
    error: &'static str,
}
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

#[derive(Serialize)]
struct MeResponse {
    discord_id: i64,
}
#[derive(Serialize)]
struct TokenResponse {
    token: String,
}
#[derive(Serialize)]
struct ServerResponse {
    discord_id: i64,
    name: String,
    icon_url: Option<String>,
}
#[derive(Serialize)]
struct ChannelResponse {
    discord_id: i64,
    name: String,
    server_id: i64,
    server_name: String,
}
#[derive(Serialize)]
struct UserResponse {
    discord_id: i64,
    username: String,
    avatar_url: Option<String>,
    first_seen_at: Option<String>,
    last_seen_at: Option<String>,
    message_count: i64,
    server_count: i64,
    channel_count: i64,
    attachment_count: i64,
    embed_count: i64,
    usernames: Vec<UserNameResponse>,
    avatars: Vec<UserAvatarResponse>,
}
#[derive(Serialize, sqlx::FromRow)]
struct UserNameResponse {
    username: String,
    first_seen_at: String,
    last_seen_at: String,
}
#[derive(Serialize, sqlx::FromRow)]
struct UserAvatarResponse {
    url: String,
    first_seen_at: String,
    last_seen_at: String,
}
#[derive(Serialize, sqlx::FromRow)]
struct AttachmentResponse {
    discord_id: i64,
    filename: String,
    description: Option<String>,
    content_type: Option<String>,
    size: i64,
}
#[derive(Serialize, sqlx::FromRow)]
struct EmbedResponse {
    index: i32,
    title: Option<String>,
    description: Option<String>,
    url: Option<String>,
}
#[derive(Serialize)]
struct MessageResponse {
    discord_id: i64,
    author_id: i64,
    author_username: String,
    server_id: i64,
    server_name: String,
    channel_id: i64,
    channel_name: String,
    timestamp: String,
    version: i64,
    archived_at: String,
    content: Option<String>,
    attachments: Vec<AttachmentResponse>,
    embeds: Vec<EmbedResponse>,
}
#[derive(Serialize, sqlx::FromRow)]
struct MessageSummary {
    discord_id: i64,
    author_id: i64,
    author_username: String,
    server_id: i64,
    server_name: String,
    channel_id: i64,
    channel_name: String,
    timestamp: String,
    content: Option<String>,
}
#[derive(Serialize)]
struct FilteredMessageSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    discord_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    server_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    server_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<Option<String>>,
}
#[derive(Serialize)]
struct SearchResponse {
    query: String,
    limit: i64,
    results: Vec<FilteredMessageSummary>,
}
#[derive(Default, Deserialize)]
struct VersionQuery {
    version: Option<i64>,
}
#[derive(Default, Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    page: Option<i64>,
    limit: Option<i64>,
    filter: Option<String>,
}

const SEARCH_FIELDS: [&str; 9] = [
    "discord_id",
    "author_id",
    "author_username",
    "server_id",
    "server_name",
    "channel_id",
    "channel_name",
    "timestamp",
    "content",
];

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MetadataType {
    #[default]
    Message,
    Server,
    Channel,
    User,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MetadataLookup {
    #[default]
    MessageCount,
    FirstMessage,
    LastMessage,
    AttachmentCount,
    EmbedCount,
    // For channels or servers
    UserCount,
    // For servers
    ChannelCount,
}

impl MetadataLookup {
    fn supports(self, metadata_type: MetadataType) -> bool {
        match self {
            Self::UserCount => {
                matches!(metadata_type, MetadataType::Server | MetadataType::Channel)
            }
            Self::ChannelCount => matches!(metadata_type, MetadataType::Server),
            _ => true,
        }
    }
}

#[derive(Default, Deserialize)]
struct MetadataQuery {
    #[serde(default)]
    mtype: MetadataType,
    id: i64,
    ltype: MetadataLookup,
}

pub(super) fn router(data: WebData) -> Router {
    let protected = Router::new()
        .route("/me", get(me))
        .route("/view/message/{id}", get(view_message))
        .route("/view/server/{id}", get(view_server))
        .route("/view/channel/{id}", get(view_channel))
        .route("/view/user/{id}", get(view_user))
        // Added attachement and attachment because I make the mistake often
        .route("/view/attachment/{id}", get(view_attachment))
        .route("/view/attachement/{id}", get(view_attachment))
        .route("/download/attachment/{id}", get(download_attachment))
        .route("/download/attachement/{id}", get(download_attachment))
        .route("/search/timestamp", get(search_timestamp))
        .route("/search/content", get(search_content))
        .route("/metadata", get(metadata_lookup))
        .route_layer(middleware::from_fn_with_state(
            data.clone(),
            require_api_user,
        ));
    Router::new()
        .route("/token", post(create_token))
        .merge(protected)
        .with_state(data)
}

async fn require_api_user(
    State(data): State<WebData>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(token) = bearer_token(request.headers().get(header::AUTHORIZATION)) else {
        return api_error(StatusCode::UNAUTHORIZED, "invalid or missing bearer token");
    };
    let discord_id =
        match sqlx::query_scalar::<_, i64>("SELECT discord_id FROM api_tokens WHERE token = $1")
            .bind(token)
            .fetch_optional(&data.pool)
            .await
        {
            Ok(Some(id)) => id,
            Ok(None) => {
                return api_error(StatusCode::UNAUTHORIZED, "invalid or missing bearer token");
            }
            Err(error) => {
                tracing::error!("API token lookup failed: {}", error);
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, "database error");
            }
        };
    let channel_ids = match cached_accessible_channels(&data, discord_id).await {
        Ok(access) => access
            .into_iter()
            .map(|channel| channel.channel_id)
            .collect(),
        Err(error) => {
            tracing::error!("API permission refresh failed: {}", error);
            return api_error(
                StatusCode::BAD_GATEWAY,
                "could not refresh Discord permissions",
            );
        }
    };
    request.extensions_mut().insert(ApiUser {
        discord_id,
        channel_ids,
    });
    next.run(request).await
}

async fn create_token(
    State(data): State<WebData>,
    session: Session,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let user = match session.get::<WebUser>("user").await {
        Ok(Some(user)) => user,
        Ok(None) => return api_error(StatusCode::UNAUTHORIZED, "browser login required"),
        Err(_) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "session error"),
    };
    let client_ip = data.token_rate_limiter.client_ip(peer.ip(), &headers);
    if let Err(retry_after) = data.token_rate_limiter.check(client_ip) {
        return rate_limited(retry_after);
    }
    let token: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();
    let mut transaction = match data.pool.begin().await {
        Ok(transaction) => transaction,
        Err(error_value) => return database(error_value).into_response(),
    };
    let result = sqlx::query(
        "INSERT INTO discord_users (discord_id, discord_username)
         VALUES ($1, $2)
         ON CONFLICT (discord_id) DO UPDATE
         SET discord_username = EXCLUDED.discord_username",
    )
    .bind(user.id)
    .bind(&user.username)
    .execute(&mut *transaction)
    .await;
    if let Err(error_value) = result {
        return database(error_value).into_response();
    }
    let result = sqlx::query("INSERT INTO api_tokens (token, discord_id) VALUES ($1, $2)")
        .bind(&token)
        .bind(user.id)
        .execute(&mut *transaction)
        .await;
    if let Err(error_value) = result {
        return database(error_value).into_response();
    }
    if let Err(error_value) = transaction.commit().await {
        return database(error_value).into_response();
    }
    Json(TokenResponse { token }).into_response()
}

async fn me(Extension(user): Extension<ApiUser>) -> Json<MeResponse> {
    Json(MeResponse {
        discord_id: user.discord_id,
    })
}

async fn view_server(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
) -> ApiResult<ServerResponse> {
    let row = sqlx::query_as::<_, (i64, String, Option<String>)>(
        "SELECT DISTINCT g.guild_id, g.guild_name, g.guild_icon_url FROM guilds g
         JOIN channels c ON c.guild_id = g.guild_id
         WHERE g.guild_id = $1 AND c.channel_id = ANY($2)",
    )
    .bind(id)
    .bind(&user.channel_ids)
    .fetch_optional(&data.pool)
    .await
    .map_err(database)?
    .ok_or_else(not_found)?;
    Ok(Json(ServerResponse {
        discord_id: row.0,
        name: row.1,
        icon_url: row.2,
    }))
}

async fn view_attachment(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
    Query(query): Query<VersionQuery>,
) -> ApiResult<AttachmentResponse> {
    let row = sqlx::query_as::<_, AttachmentResponse>(
        "SELECT attachment_id AS discord_id, filename, description, content_type, size FROM attachments a
         JOIN messages m ON m.message_id = a.message_id
         WHERE a.attachment_id = $1 AND (m.channel_id = ANY($2) OR m.author_id = $3)
           AND ($4::bigint IS NULL OR a.message_version = $4)
         ORDER BY a.message_version DESC
         LIMIT 1",
    )
    .bind(id)
    .bind(&user.channel_ids)
    .bind(user.discord_id)
    .bind(query.version)
    .fetch_optional(&data.pool)
    .await
    .map_err(database)?
    .ok_or_else(not_found)?;
    Ok(Json(row))
}

async fn download_attachment(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
    Query(query): Query<VersionQuery>,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let row = sqlx::query_as::<_, (String, Option<String>, Vec<u8>)>(
        "SELECT a.filename, a.content_type, a.data FROM attachments a
         JOIN messages m ON m.message_id = a.message_id
         WHERE a.attachment_id = $1 AND (m.channel_id = ANY($2) OR m.author_id = $3)
           AND ($4::bigint IS NULL OR a.message_version = $4)
         ORDER BY a.message_version DESC
         LIMIT 1",
    )
    .bind(id)
    .bind(&user.channel_ids)
    .bind(user.discord_id)
    .bind(query.version)
    .fetch_optional(&data.pool)
    .await
    .map_err(database)?
    .ok_or_else(not_found)?;

    let content_type = row
        .1
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", safe_filename(&row.0)),
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        row.2,
    )
        .into_response())
}

async fn view_channel(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
) -> ApiResult<ChannelResponse> {
    let row = sqlx::query_as::<_, (i64, String, i64, String)>(
        "SELECT c.channel_id, c.channel_name, g.guild_id, g.guild_name FROM channels c
         JOIN guilds g ON g.guild_id = c.guild_id WHERE c.channel_id = $1 AND c.channel_id = ANY($2)")
        .bind(id).bind(&user.channel_ids).fetch_optional(&data.pool).await.map_err(database)?.ok_or_else(not_found)?;
    Ok(Json(ChannelResponse {
        discord_id: row.0,
        name: row.1,
        server_id: row.2,
        server_name: row.3,
    }))
}

async fn view_user(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
) -> ApiResult<UserResponse> {
    let row = sqlx::query_as::<_, (i64, String)>(
        "SELECT u.discord_id, u.discord_username FROM discord_users u WHERE u.discord_id = $1
         AND (u.discord_id = $2 OR EXISTS (SELECT 1 FROM messages m
              WHERE m.author_id = u.discord_id AND m.channel_id = ANY($3)))",
    )
    .bind(id)
    .bind(user.discord_id)
    .bind(&user.channel_ids)
    .fetch_optional(&data.pool)
    .await
    .map_err(database)?
    .ok_or_else(not_found)?;
    let history = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
        "SELECT
             (ARRAY_AGG(discord_avatar_url ORDER BY last_seen_at DESC)
                 FILTER (WHERE discord_avatar_url IS NOT NULL))[1],
             MIN(first_seen_at)::text,
             MAX(last_seen_at)::text
         FROM discord_user_history WHERE discord_id = $1",
    )
    .bind(id)
    .fetch_one(&data.pool)
    .await
    .map_err(database)?;
    let counts = sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(
        "SELECT COUNT(DISTINCT m.message_id), COUNT(DISTINCT m.guild_id),
                COUNT(DISTINCT m.channel_id), COUNT(DISTINCT a.uuid), COUNT(DISTINCT e.uuid)
         FROM messages m
         LEFT JOIN attachments a ON a.message_id = m.message_id
         LEFT JOIN embeds e ON e.message_id = m.message_id
         WHERE m.author_id = $1 AND (m.channel_id = ANY($2) OR m.author_id = $3)",
    )
    .bind(id)
    .bind(&user.channel_ids)
    .bind(user.discord_id)
    .fetch_one(&data.pool)
    .await
    .map_err(database)?;
    let usernames = sqlx::query_as::<_, UserNameResponse>(
        "SELECT discord_username AS username, MIN(first_seen_at)::text AS first_seen_at,
                MAX(last_seen_at)::text AS last_seen_at
         FROM discord_user_history WHERE discord_id = $1
         GROUP BY discord_username ORDER BY MIN(first_seen_at), discord_username",
    )
    .bind(id)
    .fetch_all(&data.pool)
    .await
    .map_err(database)?;
    let avatars = sqlx::query_as::<_, UserAvatarResponse>(
        "SELECT discord_avatar_url AS url, MIN(first_seen_at)::text AS first_seen_at,
                MAX(last_seen_at)::text AS last_seen_at
         FROM discord_user_history
         WHERE discord_id = $1 AND discord_avatar_url IS NOT NULL
         GROUP BY discord_avatar_url ORDER BY MIN(first_seen_at), discord_avatar_url",
    )
    .bind(id)
    .fetch_all(&data.pool)
    .await
    .map_err(database)?;
    Ok(Json(UserResponse {
        discord_id: row.0,
        username: row.1,
        avatar_url: history.0,
        first_seen_at: history.1,
        last_seen_at: history.2,
        message_count: counts.0,
        server_count: counts.1,
        channel_count: counts.2,
        attachment_count: counts.3,
        embed_count: counts.4,
        usernames,
        avatars,
    }))
}

async fn view_message(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Path(id): Path<i64>,
    Query(query): Query<VersionQuery>,
) -> ApiResult<MessageResponse> {
    let message = sqlx::query_as::<_, (i64, i64, String, i64, String, i64, String, String)>(
        "SELECT m.message_id, m.author_id, m.author_username, m.guild_id, g.guild_name,
                m.channel_id, c.channel_name, m.timestamp::text FROM messages m
         JOIN guilds g ON g.guild_id = m.guild_id JOIN channels c ON c.channel_id = m.channel_id
         WHERE m.message_id = $1 AND (m.channel_id = ANY($2) OR m.author_id = $3)",
    )
    .bind(id)
    .bind(&user.channel_ids)
    .bind(user.discord_id)
    .fetch_optional(&data.pool)
    .await
    .map_err(database)?
    .ok_or_else(not_found)?;
    let version = sqlx::query_as::<_, (i64, Option<String>, String)>(
        "SELECT version, content, archived_at::text FROM message_versions
         WHERE message_id = $1 AND ($2::bigint IS NULL OR version = $2) ORDER BY version DESC LIMIT 1")
        .bind(id).bind(query.version).fetch_optional(&data.pool).await.map_err(database)?.ok_or_else(not_found)?;
    let attachments = sqlx::query_as::<_, AttachmentResponse>(
        "SELECT attachment_id AS discord_id, filename, description, content_type, size FROM attachments
         WHERE message_id = $1 AND message_version = $2 ORDER BY attachment_id")
        .bind(id).bind(version.0).fetch_all(&data.pool).await.map_err(database)?;
    let embeds = sqlx::query_as::<_, EmbedResponse>(
        "SELECT embed_index AS index, title, description, url FROM embeds
         WHERE message_id = $1 AND message_version = $2 ORDER BY embed_index",
    )
    .bind(id)
    .bind(version.0)
    .fetch_all(&data.pool)
    .await
    .map_err(database)?;
    Ok(Json(MessageResponse {
        discord_id: message.0,
        author_id: message.1,
        author_username: message.2,
        server_id: message.3,
        server_name: message.4,
        channel_id: message.5,
        channel_name: message.6,
        timestamp: message.7,
        version: version.0,
        content: version.1,
        archived_at: version.2,
        attachments,
        embeds,
    }))
}

async fn search_timestamp(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<SearchResponse> {
    search(&data, &user, &query, &query.limit.unwrap_or(100), true).await
}
async fn search_content(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<SearchResponse> {
    search(&data, &user, &query, &query.limit.unwrap_or(100), false).await
}
async fn search(
    data: &WebData,
    user: &ApiUser,
    query: &SearchQuery,
    limit: &i64,
    timestamp: bool,
) -> ApiResult<SearchResponse> {
    let limit = limit.max(&1);
    let pattern = format!("%{}%", query.q);
    let fields = parse_search_filter(query.filter.as_deref())?;

    let results = sqlx::query_as::<_, MessageSummary>(
        "SELECT m.message_id AS discord_id, m.author_id, m.author_username, m.guild_id AS server_id,
         g.guild_name AS server_name, m.channel_id, c.channel_name, m.timestamp::text AS timestamp, m.content
         FROM messages m
         JOIN guilds g ON g.guild_id = m.guild_id
         JOIN channels c ON c.channel_id = m.channel_id
         WHERE CASE
             WHEN $2 THEN m.timestamp::text ILIKE $1
             ELSE COALESCE(m.content, '') ILIKE $1
         END
         AND (m.channel_id = ANY($3) OR m.author_id = $4)
         ORDER BY m.timestamp DESC, m.message_id DESC
         LIMIT $5"
    )
    .bind(&pattern)
    .bind(timestamp)
    .bind(&user.channel_ids)
    .bind(user.discord_id)
    .bind(limit)
    .fetch_all(&data.pool)
    .await
    .map_err(database)?;

    let results = results
        .into_iter()
        .map(|result| FilteredMessageSummary {
            discord_id: fields[0].then_some(result.discord_id),
            author_id: fields[1].then_some(result.author_id),
            author_username: fields[2].then_some(result.author_username),
            server_id: fields[3].then_some(result.server_id),
            server_name: fields[4].then_some(result.server_name),
            channel_id: fields[5].then_some(result.channel_id),
            channel_name: fields[6].then_some(result.channel_name),
            timestamp: fields[7].then_some(result.timestamp),
            content: fields[8].then_some(result.content),
        })
        .collect();

    Ok(Json(SearchResponse {
        query: query.q.clone(),
        limit: *limit,
        results,
    }))
}

fn parse_search_filter(
    filter: Option<&str>,
) -> Result<[bool; 9], (StatusCode, Json<ErrorResponse>)> {
    let Some(filter) = filter else {
        return Ok([true; 9]);
    };
    let mut selected = [false; 9];
    for field in filter.split(',').map(str::trim) {
        let Some(index) = SEARCH_FIELDS
            .iter()
            .position(|candidate| *candidate == field)
        else {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid search filter field",
            ));
        };
        selected[index] = true;
    }
    Ok(selected)
}

async fn metadata_lookup(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Query(query): Query<MetadataQuery>,
) -> ApiResult<i64> {
    if !query.ltype.supports(query.mtype) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "metadata lookup is not supported for this type",
        ));
    }
    let entity_filter = match query.mtype {
        MetadataType::Message => "m.message_id = $1",
        MetadataType::Server => "m.guild_id = $1",
        MetadataType::Channel => "m.channel_id = $1",
        MetadataType::User => "m.author_id = $1",
    };
    let aggregate = match query.ltype {
        MetadataLookup::MessageCount => "COUNT(*)",
        MetadataLookup::FirstMessage => {
            "(ARRAY_AGG(message_id ORDER BY timestamp ASC, message_id ASC))[1]"
        }
        MetadataLookup::LastMessage => {
            "(ARRAY_AGG(message_id ORDER BY timestamp DESC, message_id DESC))[1]"
        }
        MetadataLookup::AttachmentCount => {
            "(SELECT COUNT(*) FROM attachments a WHERE a.message_id = visible.message_id)"
        }
        MetadataLookup::EmbedCount => {
            "(SELECT COUNT(*) FROM embeds e WHERE e.message_id = visible.message_id)"
        }
        MetadataLookup::UserCount => "COUNT(DISTINCT author_id)",
        MetadataLookup::ChannelCount => "COUNT(DISTINCT channel_id)",
    };

    let statement = if matches!(
        query.ltype,
        MetadataLookup::AttachmentCount | MetadataLookup::EmbedCount
    ) {
        format!(
            "SELECT COALESCE(SUM({aggregate}), 0)::bigint FROM (
                 SELECT m.message_id, m.author_id, m.channel_id, m.timestamp
                 FROM messages m
                 WHERE {entity_filter}
                   AND (m.channel_id = ANY($2) OR m.author_id = $3)
             ) visible"
        )
    } else {
        format!(
            "SELECT {aggregate} FROM (
                 SELECT m.message_id, m.author_id, m.channel_id, m.timestamp
                 FROM messages m
                 WHERE {entity_filter}
                   AND (m.channel_id = ANY($2) OR m.author_id = $3)
             ) visible"
        )
    };

    let value = sqlx::query_scalar::<_, Option<i64>>(sqlx::AssertSqlSafe(statement))
        .bind(query.id)
        .bind(&user.channel_ids)
        .bind(user.discord_id)
        .fetch_one(&data.pool)
        .await
        .map_err(database)?
        .ok_or_else(not_found)?;

    Ok(Json(value))
}

fn bearer_token(value: Option<&axum::http::HeaderValue>) -> Option<&str> {
    let value = value?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("Bearer")
        && !token.is_empty()
        && !token.contains(char::is_whitespace))
    .then_some(token)
}
fn database(error_value: sqlx::Error) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!("API database query failed: {}", error_value);
    error(StatusCode::INTERNAL_SERVER_ERROR, "database error")
}
fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    error(StatusCode::NOT_FOUND, "not found")
}
fn error(status: StatusCode, message: &'static str) -> (StatusCode, Json<ErrorResponse>) {
    (status, Json(ErrorResponse { error: message }))
}
fn api_error(status: StatusCode, message: &'static str) -> Response {
    error(status, message).into_response()
}

fn rate_limited(retry_after: u64) -> Response {
    let mut response = api_error(
        StatusCode::TOO_MANY_REQUESTS,
        "token creation rate limit exceeded",
    );
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from_str(&retry_after.to_string()).unwrap(),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(trusted_proxies: &[&str]) -> TokenRateLimiter {
        TokenRateLimiter {
            attempts: Arc::new(Mutex::new(HashMap::new())),
            trusted_proxies: Arc::new(
                trusted_proxies
                    .iter()
                    .map(|range| range.parse().unwrap())
                    .collect(),
            ),
        }
    }
    #[test]
    fn parses_bearer_tokens() {
        let header = axum::http::HeaderValue::from_static("Bearer secret-token");
        assert_eq!(bearer_token(Some(&header)), Some("secret-token"));
        let lowercase = axum::http::HeaderValue::from_static("bearer another-token");
        assert_eq!(bearer_token(Some(&lowercase)), Some("another-token"));
    }
    #[test]
    fn rejects_invalid_authorization_headers() {
        let basic = axum::http::HeaderValue::from_static("Basic credentials");
        let empty = axum::http::HeaderValue::from_static("Bearer ");
        let spaced = axum::http::HeaderValue::from_static("Bearer two tokens");
        assert_eq!(bearer_token(None), None);
        assert_eq!(bearer_token(Some(&basic)), None);
        assert_eq!(bearer_token(Some(&empty)), None);
        assert_eq!(bearer_token(Some(&spaced)), None);
    }

    #[test]
    fn search_filter_selects_requested_fields() {
        let selected = parse_search_filter(Some("discord_id, content"));
        assert_eq!(
            selected.ok(),
            Some([true, false, false, false, false, false, false, false, true])
        );
        assert_eq!(parse_search_filter(None).ok(), Some([true; 9]));
    }

    #[test]
    fn search_filter_rejects_unknown_and_empty_fields() {
        assert!(parse_search_filter(Some("unknown")).is_err());
        assert!(parse_search_filter(Some("")).is_err());
    }

    #[test]
    fn only_trusts_forwarded_ip_from_configured_proxies() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static("203.0.113.10"));
        let limiter = limiter(&["10.0.0.0/8"]);

        assert_eq!(
            limiter.client_ip("10.1.2.3".parse().unwrap(), &headers),
            "203.0.113.10".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            limiter.client_ip("192.0.2.5".parse().unwrap(), &headers),
            "192.0.2.5".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn limits_repeated_token_creation_by_ip() {
        let limiter = limiter(&[]);
        let ip = "192.0.2.5".parse().unwrap();
        assert_eq!(limiter.check(ip), Ok(()));
        assert!(limiter.check(ip).is_err());
        assert_eq!(limiter.check("192.0.2.6".parse().unwrap()), Ok(()));
    }

    #[test]
    fn caches_api_permissions_by_user_id_until_the_ttl_expires() {
        let cache = ApiPermissionCache::default();
        let refreshed_at = Instant::now();
        let access = vec![ChannelAccess {
            channel_id: 10,
            reason: "test".into(),
        }];

        cache.insert_at(1, access, refreshed_at);

        assert_eq!(
            cache
                .get_at(1, refreshed_at + Duration::from_secs(299))
                .unwrap()[0]
                .channel_id,
            10
        );
        assert!(
            cache
                .get_at(1, refreshed_at + Duration::from_secs(300))
                .is_none()
        );
    }
}
