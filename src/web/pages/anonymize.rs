use super::*;

pub(in crate::web) async fn anonymize_confirmation() -> Html<String> {
    let csrf_token = csrf_token();
    let body = render_template(&AnonymizeConfirmationTemplate {
        csrf_token: &csrf_token,
    });
    Html(page("Confirm anonymization", &body))
}

pub(in crate::web) async fn anonymize_all(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    session: Session,
    Form(form): Form<CsrfForm>,
) -> Response {
    if !csrf_token_is_valid(&form.csrf_token) {
        return csrf_error().into_response();
    }
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
