use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/search/timestamp",
    params(SearchQuery),
    responses(
        (status = 200, body = SearchResponse),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub(super) async fn search_timestamp(
    State(data): State<WebData>,
    Extension(user): Extension<ApiUser>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<SearchResponse> {
    search(&data, &user, &query, &query.limit.unwrap_or(100), true).await
}
