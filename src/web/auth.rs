use super::*;
use rand::RngExt;

pub(super) async fn timezone_request(jar: CookieJar, request: Request, next: Next) -> Response {
    let timezone = jar
        .get("timezone")
        .and_then(|cookie| cookie.value().parse::<chrono_tz::Tz>().ok());
    let detect = timezone.is_none() && request.uri().path() == "/";
    ACTIVE_TIMEZONE
        .scope(
            TimezoneContext {
                timezone: timezone.unwrap_or(chrono_tz::UTC),
                detect,
            },
            next.run(request),
        )
        .await
}

pub(super) async fn theme_request(jar: CookieJar, request: Request, next: Next) -> Response {
    let theme = Theme::from_cookie(jar.get("theme").map(Cookie::value));
    ACTIVE_THEME.scope(theme, next.run(request)).await
}

pub(super) async fn require_user(
    State(data): State<WebData>,
    session: Session,
    mut request: Request,
    next: Next,
) -> Response {
    match session.get::<WebUser>("user").await {
        Ok(Some(mut user)) => {
            if channel_access_needs_refresh(&user, request.uri().path()) {
                let channel_access = match accessible_channels(&data, user.id).await {
                    Ok(channel_access) => channel_access,
                    Err(error) => {
                        error!("Discord permission refresh failed: {}", error);
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                };
                user.channel_ids = channel_access
                    .iter()
                    .map(|access| access.channel_id)
                    .collect();
                user.channel_access = channel_access;
                user.channel_access_refreshed_at = unix_timestamp();
                if let Err(error) = session.insert("user", &user).await {
                    error!("Could not update refreshed session permissions: {}", error);
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
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

pub(super) fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn channel_access_needs_refresh_at(user: &WebUser, path: &str, now: u64) -> bool {
    let expired =
        now.saturating_sub(user.channel_access_refreshed_at) >= CHANNEL_ACCESS_TTL_SECONDS;
    let requested_channel_is_missing = path
        .trim_matches('/')
        .split('/')
        .collect::<Vec<_>>()
        .as_slice()
        .get(0..2)
        .and_then(|parts| match parts {
            ["channels", id] => id.parse::<i64>().ok(),
            _ => None,
        })
        .is_some_and(|channel_id| !user.channel_ids.contains(&channel_id));
    expired || requested_channel_is_missing
}

pub(super) fn channel_access_needs_refresh(user: &WebUser, path: &str) -> bool {
    channel_access_needs_refresh_at(user, path, unix_timestamp())
}

pub(super) async fn request_is_allowed(pool: &PgPool, user: &WebUser, path: &str) -> bool {
    let parts = path.trim_matches('/').split('/').collect::<Vec<_>>();
    let channel_id = match parts.as_slice() {
        ["channels", id, ..] => id.parse::<i64>().ok(),
        ["messages", id] => return match id.parse::<i64>() {
            Ok(id) => sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(
                    SELECT 1 FROM messages
                    WHERE message_id = $1
                      AND (author_id = $2 OR channel_id = ANY($3))
                )",
            )
            .bind(id)
            .bind(user.id)
            .bind(&user.channel_ids)
            .fetch_one(pool)
            .await
            .unwrap_or(false),
            Err(_) => false,
        },
        ["attachments", id] => return match id.parse::<i64>() {
            Ok(id) => sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(
                    SELECT 1 FROM attachments a
                    JOIN messages m ON m.message_id = a.message_id
                    WHERE a.attachment_id = $1
                      AND (m.author_id = $2 OR m.channel_id = ANY($3))
                )",
            )
            .bind(id)
            .bind(user.id)
            .bind(&user.channel_ids)
            .fetch_one(pool)
            .await
            .unwrap_or(false),
            Err(_) => false,
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
            Ok(id) if id == user.id => true,
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

pub(super) async fn login(session: Session, Query(query): Query<LoginQuery>) -> Response {
    match session.get::<WebUser>("user").await {
        Ok(Some(_)) => Redirect::to("/").into_response(),
        Ok(None) => Html(render_page(
            "Log in",
            &render_template(&LoginTemplate {
                error: query.error.as_deref(),
            }),
            false,
        ))
        .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(super) async fn privacy(session: Session) -> Response {
    match session.get::<WebUser>("user").await {
        Ok(user) => {
            let logged_in = user.is_some();
            let body = render_template(&PrivacyTemplate { logged_in });
            Html(render_page("Privacy", &body, logged_in)).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(super) async fn privacy_policy(session: Session) -> Response {
    match session.get::<WebUser>("user").await {
        Ok(user) => Html(render_page(
            "Privacy policy",
            &privacy_policy_html(),
            user.is_some(),
        ))
        .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn privacy_policy_html() -> String {
    const MARKDOWN: &str = include_str!("../../static/privacy-policy.md");

    let options = pulldown_cmark::Options::all();
    let parser = pulldown_cmark::Parser::new_ext(MARKDOWN, options);

    let mut output = String::new();
    pulldown_cmark::html::push_html(&mut output, parser);

    output
}

pub(super) async fn anonymize_confirmation() -> Html<String> {
    let body = render_template(&AnonymizeConfirmationTemplate);
    Html(page("Confirm anonymization", &body))
}

pub(super) async fn anonymize_all(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    session: Session,
) -> Response {
    let result = async {
        let mut transaction = data.pool.begin().await?;
        sqlx::query(
            "INSERT INTO discord_users (discord_id, discord_username)
             VALUES (0, 'Deleted user')
             ON CONFLICT (discord_id) DO UPDATE SET discord_username = EXCLUDED.discord_username",
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO discord_user_history
                 (discord_id, discord_username, discord_avatar_url)
             VALUES (0, 'Deleted user', NULL)
             ON CONFLICT (discord_id, discord_username, discord_avatar_url)
             DO UPDATE SET last_seen_at = EXCLUDED.last_seen_at",
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO guild_users (guild_id, discord_id, first_seen_at, last_seen_at)
             SELECT guild_id, 0, first_seen_at, last_seen_at
             FROM guild_users
             WHERE discord_id = $1
             ON CONFLICT (guild_id, discord_id) DO UPDATE SET
                 first_seen_at = LEAST(guild_users.first_seen_at, EXCLUDED.first_seen_at),
                 last_seen_at = GREATEST(guild_users.last_seen_at, EXCLUDED.last_seen_at)",
        )
        .bind(user.id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE messages
             SET author_id = 0, author_username = 'Deleted user'
             WHERE author_id = $1",
        )
        .bind(user.id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM discord_users WHERE discord_id = $1")
            .bind(user.id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await
    }
    .await;

    if let Err(error) = result {
        error!("Could not anonymize Discord user {}: {}", user.id, error);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let _ = session.delete().await;
    Redirect::to("/privacy").into_response()
}

pub(super) async fn set_theme(
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

pub(super) async fn theme_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../static/theme.css"),
    )
}

pub(super) async fn timezone_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/timezone.js"),
    )
}

pub(super) async fn set_timezone(
    State(data): State<WebData>,
    jar: CookieJar,
    Json(form): Json<TimezoneForm>,
) -> Response {
    if form.timezone.parse::<chrono_tz::Tz>().is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let cookie = Cookie::build(("timezone", form.timezone))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(!data.redirect_uri.starts_with("http://"))
        .max_age(Duration::days(365))
        .build();
    (jar.add(cookie), StatusCode::NO_CONTENT).into_response()
}

pub(super) async fn discord_login(State(data): State<WebData>, session: Session) -> Response {
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

pub(super) async fn discord_callback(
    State(data): State<WebData>,
    session: Session,
    Query(query): Query<OAuthQuery>,
) -> Response {
    if !query.code.is_some() {
        return oauth2_error_handling(query.error.as_deref().unwrap_or("none"));
    }
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
            ("code", query.code.expect("Missing code in query").as_str()),
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
        channel_access_refreshed_at: unix_timestamp(),
    };
    if session.cycle_id().await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if session.insert("user", user).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    Redirect::to("/").into_response()
}

fn oauth2_error_handling(error_code: &str) -> Response {
    return match error_code {
        "access_denied" => Redirect::to(
            "/login?error=You denied the request. Please log in again and hit Authorize to continue."
        ).into_response(),

        "invalid_request" => Redirect::to(
            "/login?error=Error code: invalid_request. Please try again. If this keeps happening, report this issue."
        ).into_response(),

        "unauthorized_client" => Redirect::to(
            "/login?error=Error code: unauthorized_client. Please try again. If this keeps happening, report this issue."
        ).into_response(),

        "unsupported_response_type" => Redirect::to(
            "/login?error=Error code: unsupported_response_type. Please try again. If this keeps happening, report this issue."
        ).into_response(),

        "invalid_scope" => Redirect::to(
            "/login?error=Error code: invalid_scope. Please try again. If this keeps happening, report this issue. Also if this is just you messing with the oauth2 scope, stop."
        ).into_response(),

        "server_error" => Redirect::to(
            "/login?error=Discord encountered an internal error. Please try again."
        ).into_response(),

        "temporarily_unavailable" => Redirect::to(
            "/login?error=Discord authentication is temporarily unavailable. Please try again later and check discordstatus.com for updates."
        ).into_response(),

        "none" => Redirect::to(
            "/login?error=Error code: none. Please try again. If this keeps happening, report this issue. Also if this is just you messing with the oauth2 scope, stop."
        ).into_response(),

        _ =>  Redirect::to(
            "/login?error=Error code: unknown. Please try again. If this keeps happening, report this issue."
        ).into_response(),
    };
}

pub(super) async fn logout(session: Session) -> Redirect {
    let _ = session.delete().await;
    Redirect::to("/login")
}

pub(super) fn oauth_error(message: &str) -> Response {
    (StatusCode::BAD_GATEWAY, message.to_string()).into_response()
}

pub(super) async fn accessible_channels(
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

pub(super) async fn discord_get<T: serde::de::DeserializeOwned>(
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

pub(super) async fn discord_get_optional<T: serde::de::DeserializeOwned>(
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

pub(super) fn can_view_channel(
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

pub(super) fn view_channel_reason(
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

pub(super) fn apply_overwrite(
    permissions: &mut u64,
    overwrites: &[DiscordOverwrite],
    id: i64,
    kind: u8,
) {
    let overwrite = overwrites
        .iter()
        .find(|overwrite| overwrite.kind == kind && overwrite.id.parse::<i64>() == Ok(id));
    if let Some(overwrite) = overwrite {
        *permissions &= !overwrite.deny.parse::<u64>().unwrap_or_default();
        *permissions |= overwrite.allow.parse::<u64>().unwrap_or_default();
    }
}
