use super::{
    CHANNEL_ACCESS_TTL_SECONDS, ChannelAccess, WebData, WebUser, accessible_channels, safe_filename,
};
use axum::{
    Extension, Json, Router, extract::{ConnectInfo, Path, Query, Request, State}, http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header}, middleware::{self, Next}, response::{IntoResponse, Response}, routing::{get, post},
};
use rand::{RngExt, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tower_sessions::Session;
use tower_sessions_redis_store::fred::{error::{Error as RedisError, ErrorKind as RedisErrorKind}, prelude::*, types::{Expiration, SetOptions}};
use utoipa::{IntoParams, OpenApi, ToSchema};

mod create_token;
mod token_info;
mod revoke_token;
mod me;
mod view_server;
mod view_attachment;
mod download_attachment;
mod view_channel;
mod view_user;
mod view_message;
mod search_timestamp;
mod search_content;
mod search;
mod metadata_lookup;
mod healthcheck;
mod search_image;

pub(super) use create_token::create_token;
use token_info::token_info;
use revoke_token::revoke_token;
use me::me;
use view_server::view_server;
use view_attachment::view_attachment;
use download_attachment::download_attachment;
use view_channel::view_channel;
use view_user::view_user;
use view_message::view_message;
use search_timestamp::search_timestamp;
use search_content::search_content;
use search::search;
use metadata_lookup::metadata_lookup;
use healthcheck::healthcheck;
use revoke_token::invalid_method_use_post;
use search_image::search_image;

const RESULTS_PER_PAGE: i64 = 100;
const TOKEN_RATE_LIMIT: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
pub(super) struct ApiPermissionCache {
    redis: Pool,
}

impl ApiPermissionCache {
    pub(super) fn new(redis: Pool) -> Self {
        Self { redis }
    }

    fn key(discord_id: i64) -> String {
        format!("api-permissions:{discord_id}")
    }

    async fn get(&self, discord_id: i64) -> Result<Option<Vec<ChannelAccess>>, RedisError> {
        let value = self.redis.get::<Option<String>, _>(Self::key(discord_id)).await?;
        value
            .map(|value| serde_json::from_str(&value).map_err(|error| RedisError::new(RedisErrorKind::Parse, error.to_string())))
            .transpose()
    }

    async fn insert(&self, discord_id: i64, channel_access: &[ChannelAccess]) -> Result<(), RedisError> {
        let value = serde_json::to_string(channel_access)
            .map_err(|error| RedisError::new(RedisErrorKind::Parse, error.to_string()))?;
        self.redis.set(
            Self::key(discord_id),
            value,
            Some(Expiration::EX(CHANNEL_ACCESS_TTL_SECONDS as i64)),
            None,
            false,
        ).await
    }
}

async fn cached_accessible_channels(
    data: &WebData,
    discord_id: i64,
) -> Result<Vec<ChannelAccess>, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(channel_access) = data.api_permission_cache.get(discord_id).await? {
        return Ok(channel_access);
    }

    let channel_access = accessible_channels(data, discord_id).await?;
    data.api_permission_cache
        .insert(discord_id, &channel_access).await?;
    Ok(channel_access)
}

#[derive(Clone)]
pub(super) struct TokenRateLimiter {
    redis: Pool,
    trusted_proxies: Arc<Vec<ipnet::IpNet>>,
}

impl TokenRateLimiter {
    fn key(ip: IpAddr) -> String {
        format!("token-rate-limit:{ip}")
    }

    pub(super) fn from_env(redis: Pool) -> Result<Self, String> {
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
            redis,
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

    async fn check(&self, ip: IpAddr) -> Result<Result<(), u64>, RedisError> {
        let key = Self::key(ip);
        let created = self.redis.set::<bool, _, _>(
            &key,
            "1",
            Some(Expiration::EX(TOKEN_RATE_LIMIT.as_secs() as i64)),
            Some(SetOptions::NX),
            false,
        ).await?;
        if !created {
            let remaining = self.redis.ttl::<i64, _>(key).await?;
            return Ok(Err(remaining.max(1) as u64));
        }
        Ok(Ok(()))
    }
}

#[derive(Clone)]
struct ApiUser {
    token_id: i64,
    discord_id: i64,
    channel_ids: Vec<i64>,
}
#[derive(Serialize, ToSchema)]
struct ErrorResponse {
    error: &'static str,
}
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

#[derive(Serialize, ToSchema)]
struct MeResponse {
    discord_id: i64,
}
#[derive(Serialize, ToSchema)]
struct TokenResponse {
    id: i64,
    token: String,
}
#[derive(Default, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub(super) struct CreateTokenQuery {
    pub(super) valid_from: Option<i64>,
    pub(super) valid_to: Option<i64>,
}
#[derive(Serialize, sqlx::FromRow, ToSchema)]
struct TokenInfoResponse {
    id: i64,
    created_at: i64,
    valid_from: Option<i64>,
    valid_to: Option<i64>,
    last_used_at: Option<i64>,
}
#[derive(Serialize, ToSchema)]
struct ServerResponse {
    discord_id: i64,
    name: String,
    icon_url: Option<String>,
}
#[derive(Serialize, ToSchema)]
struct ChannelResponse {
    discord_id: i64,
    name: String,
    server_id: i64,
    server_name: String,
}
#[derive(Serialize, ToSchema)]
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
#[derive(Serialize, sqlx::FromRow, ToSchema)]
struct UserNameResponse {
    username: String,
    first_seen_at: String,
    last_seen_at: String,
}
#[derive(Serialize, sqlx::FromRow, ToSchema)]
struct UserAvatarResponse {
    url: String,
    first_seen_at: String,
    last_seen_at: String,
}
#[derive(Serialize, sqlx::FromRow, ToSchema)]
struct AttachmentResponse {
    discord_id: i64,
    filename: String,
    description: Option<String>,
    content_type: Option<String>,
    size: i64,
}
#[derive(Serialize, sqlx::FromRow, ToSchema)]
struct EmbedResponse {
    index: i32,
    title: Option<String>,
    description: Option<String>,
    url: Option<String>,
}
#[derive(Serialize, ToSchema)]
struct MessageResponse {
    discord_id: i64,
    author_id: i64,
    author_username: String,
    server_id: i64,
    server_name: String,
    channel_id: i64,
    channel_name: String,
    timestamp: String,
    archive_incomplete: bool,
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
#[derive(Serialize, ToSchema)]
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
#[derive(Serialize, ToSchema)]
struct SearchResponse {
    query: String,
    limit: i64,
    results: Vec<FilteredMessageSummary>,
}
#[derive(Default, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
struct VersionQuery {
    version: Option<i64>,
}
#[derive(Default, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    limit: Option<i64>,
    filter: Option<String>,
    #[serde(default, deserialize_with = "deserialize_search_scope")]
    #[param(style = Form, explode = false)]
    channel_scope: Option<Vec<i64>>,
    #[serde(default, deserialize_with = "deserialize_search_scope")]
    #[param(style = Form, explode = false)]
    server_scope: Option<Vec<i64>>,
}
enum SearchType {
    Timestamp,
    Content,
    Image,
}

fn deserialize_search_scope<'de, D>(deserializer: D) -> Result<Option<Vec<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    value
        .split(',')
        .map(|value| value.trim().parse::<i64>().map_err(serde::de::Error::custom))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
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

#[derive(Clone, Copy, Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
enum MetadataType {
    #[default]
    Message,
    Server,
    Channel,
    User,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, ToSchema)]
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

#[derive(Default, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
struct MetadataQuery {
    #[serde(default)]
    mtype: MetadataType,
    id: i64,
    ltype: MetadataLookup,
}

#[derive(OpenApi)]
#[openapi(
    paths(
        create_token::create_token, token_info::token_info, revoke_token::revoke_token, me::me, view_server::view_server, view_attachment::view_attachment,
        download_attachment::download_attachment, view_channel::view_channel, view_user::view_user, view_message::view_message, search_timestamp::search_timestamp,
        search_content::search_content, search_image::search_image, metadata_lookup::metadata_lookup, healthcheck::healthcheck
    ),
    components(schemas(
        ErrorResponse, MeResponse, TokenResponse, TokenInfoResponse, ServerResponse,
        ChannelResponse, UserResponse, UserNameResponse, UserAvatarResponse,
        AttachmentResponse, EmbedResponse, MessageResponse, FilteredMessageSummary,
        SearchResponse, MetadataType, MetadataLookup
    )),
    modifiers(&SecurityAddon)
)]
pub(super) struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        openapi.components.as_mut().unwrap().add_security_scheme(
            "bearer_auth",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::Http::new(
                    utoipa::openapi::security::HttpAuthScheme::Bearer,
                ),
            ),
        );
    }
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
        .route("/search/image", get(search_image))
        .route("/metadata", get(metadata_lookup))
        .route("/token/info", get(token_info))
        .route("/token/revoke", post(revoke_token))
        .route("/token/revoke", get(invalid_method_use_post))
        .route_layer(middleware::from_fn_with_state(
            data.clone(),
            require_api_user,
        ));
    let app = Router::new()
        .route("/token", post(create_token))
        .route("/healthcheck", get(healthcheck))
        .merge(protected)
        .route_layer(middleware::from_fn_with_state(
            data.clone(),
            set_security_headers,
        ))
        .with_state(data);

    app
}

async fn require_api_user(
    State(data): State<WebData>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(token) = bearer_token(request.headers().get(header::AUTHORIZATION)) else {
        return api_error(StatusCode::UNAUTHORIZED, "invalid or missing bearer token");
    };
    let token_hash = token_hash(token);
    let identity = match sqlx::query_as::<_, (i64, i64)>(
        "UPDATE api_tokens
             SET last_used_at = NOW()
             WHERE token_hash = $1
               AND (valid_from IS NULL OR valid_from <= NOW())
               AND (valid_to IS NULL OR valid_to >= NOW())
             RETURNING id, discord_id",
    )
    .bind(token_hash)
    .fetch_optional(&data.pool)
    .await
    {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            return api_error(StatusCode::UNAUTHORIZED, "invalid or missing bearer token");
        }
        Err(error) => {
            tracing::error!("API token lookup failed: {}", error);
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "database error");
        }
    };
    let (token_id, discord_id) = identity;
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
        token_id,
        discord_id,
        channel_ids,
    });
    next.run(request).await
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn valid_unix_timestamp(value: Option<i64>) -> bool {
    value.is_none_or(|value| chrono::DateTime::from_timestamp(value, 0).is_some())
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

async fn set_security_headers(
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_image_search_with_bearer_authentication() {
        let document = serde_json::to_value(ApiDoc::openapi()).unwrap();
        let operation = &document["paths"]["/api/v1/search/image"]["get"];
        assert_eq!(operation["security"][0]["bearer_auth"], serde_json::json!([]));
        assert!(operation["responses"].get("503").is_some());
        assert!(operation["parameters"].as_array().unwrap().iter().any(|parameter| parameter["name"] == "q"));
    }

    #[test]
    fn parses_search_scopes() {
        let query = Query::<SearchQuery>::try_from_uri(
            &"/search?channel_scope=123,456&server_scope=789".parse().unwrap(),
        ).unwrap().0;
        assert_eq!(query.channel_scope, Some(vec![123, 456]));
        assert_eq!(query.server_scope, Some(vec![789]));
        let query = Query::<SearchQuery>::try_from_uri(&"/search?q=test".parse().unwrap()).unwrap().0;
        assert_eq!(query.channel_scope, None);
        assert_eq!(query.server_scope, None);
        for value in ["", "123,", "invalid", "9223372036854775808"] {
            let uri = format!("/search?channel_scope={value}").parse().unwrap();
            assert!(Query::<SearchQuery>::try_from_uri(&uri).is_err());
        }
    }

    fn limiter(trusted_proxies: &[&str]) -> TokenRateLimiter {
        TokenRateLimiter {
            redis: Pool::new(Config::default(), None, None, None, 1).unwrap(),
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
    fn hashes_tokens_deterministically_without_storing_the_secret() {
        assert_eq!(token_hash("secret-token"), token_hash("secret-token"));
        assert_ne!(token_hash("secret-token"), b"secret-token");
    }

    #[test]
    fn validates_token_validity_timestamps() {
        assert!(valid_unix_timestamp(None));
        assert!(valid_unix_timestamp(Some(0)));
        assert!(!valid_unix_timestamp(Some(i64::MAX)));
    }

    #[tokio::test]
    async fn enforces_token_ownership_validity_last_use_and_self_revocation_in_the_database() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let Ok(pool) = sqlx::PgPool::connect(&database_url).await else {
            return;
        };
        sqlx::migrate!().run(&pool).await.unwrap();
        let first_user = -9_000_000_011_i64;
        let second_user = -9_000_000_012_i64;
        sqlx::query(
            "INSERT INTO discord_users (discord_id, discord_username)
             VALUES ($1, 'first'), ($2, 'second')
             ON CONFLICT (discord_id) DO NOTHING",
        )
        .bind(first_user)
        .bind(second_user)
        .execute(&pool)
        .await
        .unwrap();
        let active_hash = token_hash("database-active-token");
        let expired_hash = token_hash("database-expired-token");
        let active_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO api_tokens (token_hash, discord_id, valid_from, valid_to)
             VALUES ($1, $2, NOW() - INTERVAL '1 minute', NOW() + INTERVAL '1 minute')
             RETURNING id",
        )
        .bind(&active_hash)
        .bind(first_user)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO api_tokens (token_hash, discord_id, valid_to)
             VALUES ($1, $2, NOW() - INTERVAL '1 minute')",
        )
        .bind(&expired_hash)
        .bind(second_user)
        .execute(&pool)
        .await
        .unwrap();

        let identity = sqlx::query_as::<_, (i64, i64)>(
            "UPDATE api_tokens
             SET last_used_at = NOW()
             WHERE token_hash = $1
               AND (valid_from IS NULL OR valid_from <= NOW())
               AND (valid_to IS NULL OR valid_to >= NOW())
             RETURNING id, discord_id",
        )
        .bind(&active_hash)
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_eq!(identity, Some((active_id, first_user)));
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT last_used_at IS NOT NULL FROM api_tokens WHERE id = $1",
            )
            .bind(active_id)
            .fetch_one(&pool)
            .await
            .unwrap()
        );
        assert!(
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT discord_id FROM api_tokens
             WHERE token_hash = $1
               AND (valid_from IS NULL OR valid_from <= NOW())
               AND (valid_to IS NULL OR valid_to >= NOW())",
            )
            .bind(&expired_hash)
            .fetch_optional(&pool)
            .await
            .unwrap()
            .is_none()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM api_tokens WHERE discord_id = $1",)
                .bind(first_user)
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query("DELETE FROM api_tokens WHERE id = $1 AND discord_id = $2")
                .bind(active_id)
                .bind(second_user)
                .execute(&pool)
                .await
                .unwrap()
                .rows_affected(),
            0
        );
        assert_eq!(
            sqlx::query("DELETE FROM api_tokens WHERE id = $1 AND discord_id = $2")
                .bind(active_id)
                .bind(first_user)
                .execute(&pool)
                .await
                .unwrap()
                .rows_affected(),
            1
        );

        sqlx::query("DELETE FROM discord_users WHERE discord_id IN ($1, $2)")
            .bind(first_user)
            .bind(second_user)
            .execute(&pool)
            .await
            .unwrap();
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
        let selected = search::parse_search_filter(Some("discord_id, content"));
        assert_eq!(
            selected.ok(),
            Some([true, false, false, false, false, false, false, false, true])
        );
        assert_eq!(search::parse_search_filter(None).ok(), Some([true; 9]));
    }

    #[test]
    fn search_filter_rejects_unknown_and_empty_fields() {
        assert!(search::parse_search_filter(Some("unknown")).is_err());
        assert!(search::parse_search_filter(Some("")).is_err());
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
    fn namespaces_token_creation_rate_limits_by_ip() {
        assert_eq!(
            TokenRateLimiter::key("192.0.2.5".parse().unwrap()),
            "token-rate-limit:192.0.2.5"
        );
    }

    #[test]
    fn serializes_api_permissions_for_the_shared_cache() {
        let access = vec![ChannelAccess {
            channel_id: 10,
            reason: "test".into(),
        }];
        let value = serde_json::to_string(&access).unwrap();
        let cached = serde_json::from_str::<Vec<ChannelAccess>>(&value).unwrap();
        assert_eq!(ApiPermissionCache::key(1), "api-permissions:1");
        assert_eq!(cached[0].channel_id, 10);
    }
}
