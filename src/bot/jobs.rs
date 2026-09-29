use tracing::{debug, error, trace, warn};

pub async fn run_jobs(pool: sqlx::PgPool) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        interval.tick().await;
        if let Err(e) = crate::bot::jobs::check_user_pfps(&pool).await {
            error!("Error checking user avatars: {}", e);
         }
    }
}

async fn check_user_pfps(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    debug!("Checking user avatars in the database...");
    let avatars = sqlx::query!(
        r#"
        SELECT uuid, discord_id, image_available, image_checked, discord_avatar_url
        FROM discord_user_history
        WHERE image_checked = false
        "#
    )
    .fetch_all(pool)
    .await?;

    for avatar in avatars {
        let discord_id = avatar.discord_id;
        let uuid = avatar.uuid;
        let image_checked = avatar.image_checked;
        let discord_avatar_url = avatar.discord_avatar_url;
        trace!("Checking avatar for user {}", discord_id);

        if !image_checked {
            let response = match &discord_avatar_url {
                Some(url) => match reqwest::get(url).await {
                    Ok(response) => Some(response),
                    Err(e) => {
                        warn!("Error checking avatar for user {}: {}", discord_id, e);
                        continue;
                    }
                },
                None => None,
            };

            if response.as_ref().is_none_or(|response| {
                response.status() == reqwest::StatusCode::NOT_FOUND
                    || response.status() == reqwest::StatusCode::GONE
            }) {
                trace!("Avatar not available for user {}", discord_id);
                sqlx::query!(
                    r#"
                    UPDATE discord_user_history
                    SET image_available = false, image_checked = true
                    WHERE uuid = $1
                    "#,
                    uuid
                )
                .execute(pool)
                .await?;
                continue;
            }

            if let Some(response) = response {
                if !response.status().is_success() {
                    warn!("Error checking avatar for user {}: {}", discord_id, response.status());
                    continue;
                }

                let image_mimetype = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);

                let image_data = match response.bytes().await {
                    Ok(image_data) => image_data.to_vec(),
                    Err(e) => {
                        warn!("Error downloading avatar for user {}: {}", discord_id, e);
                        continue;
                    }
                };

                sqlx::query!(
                    r#"
                    UPDATE discord_user_history
                    SET image_data = $1, image_available = true, image_checked = true, image_mimetype = $2
                    WHERE uuid = $3
                    "#,
                    image_data,
                    image_mimetype,
                    uuid
                )
                .execute(pool)
                .await?;




                
            }
            
            
        }
    }

        
    debug!("Finished checking user avatars.");

    Ok(())
}