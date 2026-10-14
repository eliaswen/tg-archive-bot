use super::*;
use axum::extract::ConnectInfo;
use chrono::TimeZone;

#[derive(Template)]
#[template(path = "api-tokens.html")]
struct ApiTokensTemplate<'a> {
    csrf_token: &'a str,
    timezone: chrono_tz::Tz,
    token: Option<&'a str>,
    error: Option<&'a str>,
}

#[derive(Deserialize)]
pub(in crate::web) struct ApiTokenForm {
    csrf_token: String,
    valid_from: String,
    valid_to: String,
}

fn api_tokens_page(token: Option<&str>, error: Option<&str>) -> Response {
    let body = render_template(&ApiTokensTemplate {
        csrf_token: &csrf_token(),
        timezone: ACTIVE_TIMEZONE.try_with(|context| context.timezone).unwrap_or(chrono_tz::UTC),
        token,
        error,
    });
    Html(page("API tokens", &body)).into_response()
}

pub(in crate::web) async fn api_tokens() -> Response {
    api_tokens_page(None, None)
}

pub(in crate::web) async fn generate_api_token(
    State(data): State<WebData>,
    session: Session,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<ApiTokenForm>,
) -> Response {
    if !csrf_token_is_valid(&form.csrf_token) {
        return csrf_error().into_response();
    }
    let timezone = ACTIVE_TIMEZONE.try_with(|context| context.timezone).unwrap_or(chrono_tz::UTC);
    let parse_timestamp = |value: &str| {
        if value.trim().is_empty() {
            Some(None)
        } else {
            chrono::NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%dT%H:%M:%S")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%dT%H:%M"))
                .ok()
                .and_then(|timestamp| timezone.from_local_datetime(&timestamp).single())
                .map(|timestamp| Some(timestamp.timestamp()))
        }
    };
    let (Some(valid_from), Some(valid_to)) = (
        parse_timestamp(&form.valid_from),
        parse_timestamp(&form.valid_to),
    ) else {
        let mut response = api_tokens_page(None, Some("Enter a valid, unambiguous date and time in the displayed timezone, or leave the field blank."));
        *response.status_mut() = StatusCode::BAD_REQUEST;
        return response;
    };
    let response = api::create_token(
        State(data),
        session,
        ConnectInfo(peer),
        headers,
        Query(api::CreateTokenQuery { valid_from, valid_to }),
    ).await;
    let (parts, body) = response.into_parts();
    let body = match to_bytes(body, 16 * 1024).await {
        Ok(body) => body,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let result = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(result) => result,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let mut response = if parts.status.is_success() {
        api_tokens_page(result.get("token").and_then(|value| value.as_str()), None)
    } else {
        api_tokens_page(None, Some(result.get("error").and_then(|value| value.as_str()).unwrap_or("Could not generate an API token.")))
    };
    *response.status_mut() = parts.status;
    if let Some(retry_after) = parts.headers.get(header::RETRY_AFTER) {
        response.headers_mut().insert(header::RETRY_AFTER, retry_after.clone());
    }
    response
}
