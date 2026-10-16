use tracing::{debug, error, trace, warn};

pub async fn run_jobs(pool: sqlx::PgPool) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        interval.tick().await;
        if let Err(e) = crate::bot::jobs::check_user_pfps(&pool).await {
            error!("Error checking user avatars: {}", e);
         }
        if let Err(e) = crate::bot::jobs::check_server_icons(&pool).await {
            error!("Error checking server icons: {}", e);
        }
    }
}

async fn check_user_pfps(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    debug!("Checking user avatars in the database...");
    let avatars = sqlx::query_as::<_, (sqlx::types::Uuid, i64, bool, Option<String>)>(
        "SELECT uuid, discord_id, image_checked, discord_avatar_url
         FROM discord_user_history
         WHERE image_checked = false"
    )
    .fetch_all(pool)
    .await?;

    for (uuid, discord_id, image_checked, discord_avatar_url) in avatars {
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
                sqlx::query(
                    "UPDATE discord_user_history
                     SET image_available = false, image_checked = true
                     WHERE uuid = $1"
                )
                .bind(uuid)
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

                sqlx::query(
                    "UPDATE discord_user_history
                     SET image_data = $1, image_available = true, image_checked = true, image_mimetype = $2
                     WHERE uuid = $3"
                )
                .bind(image_data)
                .bind(image_mimetype)
                .bind(uuid)
                .execute(pool)
                .await?;




                
            }
            
            
        }
    }

        
    debug!("Finished checking user avatars.");

    Ok(())
}

async fn check_server_icons(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    debug!("Checking server icons in the database...");
    let icons = sqlx::query_as::<_, (sqlx::types::Uuid, i64, bool, Option<String>)>(
        "SELECT uuid, guild_id, image_checked, guild_icon_url
         FROM guild_history
         WHERE image_checked = false"
    )
    .fetch_all(pool)
    .await?;

    for (uuid, guild_id, image_checked, guild_icon_url) in icons {
        trace!("Checking icon for server {}", guild_id);

        if !image_checked {
            let response = match &guild_icon_url {
                Some(url) => match reqwest::get(url).await {
                    Ok(response) => Some(response),
                    Err(e) => {
                        warn!("Error checking icon for server {}: {}", guild_id, e);
                        continue;
                    }
                },
                None => None,
            };

            if response.as_ref().is_none_or(|response| {
                response.status() == reqwest::StatusCode::NOT_FOUND
                    || response.status() == reqwest::StatusCode::GONE
            }) {
                trace!("Icon not available for server {}", guild_id);
                sqlx::query(
                    "UPDATE guild_history
                     SET image_available = false, image_checked = true
                     WHERE uuid = $1"
                )
                .bind(uuid)
                .execute(pool)
                .await?;
                continue;
            }

            if let Some(response) = response {
                if !response.status().is_success() {
                    warn!("Error checking icon for server {}: {}", guild_id, response.status());
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
                        warn!("Error downloading icon for server {}: {}", guild_id, e);
                        continue;
                    }
                };

                sqlx::query(
                    "UPDATE guild_history
                     SET image_data = $1, image_available = true, image_checked = true, image_mimetype = $2
                     WHERE uuid = $3"
                )
                .bind(image_data)
                .bind(image_mimetype)
                .bind(uuid)
                .execute(pool)
                .await?;

            }

        }
    }

    debug!("Finished checking server icons.");

    Ok(())
}
