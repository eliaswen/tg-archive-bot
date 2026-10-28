use super::*;

pub(in crate::web) async fn privacy(session: Session) -> Response {
    match session.get::<WebUser>("user").await {
        Ok(user) => {
            let logged_in = user.is_some();
            let body = render_template(&PrivacyTemplate { logged_in });
            Html(render_page("Privacy", &body, logged_in)).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
