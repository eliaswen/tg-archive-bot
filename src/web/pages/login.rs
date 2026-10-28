use super::*;

pub(in crate::web) async fn login(session: Session, Query(query): Query<LoginQuery>) -> Response {
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
