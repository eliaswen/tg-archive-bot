use super::*;

pub(in crate::web) async fn privacy_policy(session: Session) -> Response {
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
    const MARKDOWN: &str = include_str!("../../../static/privacy-policy.md");

    let options = pulldown_cmark::Options::all();
    let parser = pulldown_cmark::Parser::new_ext(MARKDOWN, options);

    let mut output = String::new();
    pulldown_cmark::html::push_html(&mut output, parser);

    output
}
