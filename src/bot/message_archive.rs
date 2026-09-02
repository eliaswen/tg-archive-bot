use crate::Error;
use poise::serenity_prelude as sere;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tracing::{debug, error, trace};

const MAX_ARCHIVE_ATTEMPTS: i32 = 10;

#[derive(serde::Deserialize, serde::Serialize)]
enum ArchivePayload {
    Message(Box<sere::Message>),
    Update {
        event: Box<sere::MessageUpdateEvent>,
        guild_id: Option<u64>,
    },
}

pub fn ensure_worker_started(ctx: &sere::Context, data: &crate::Data) {
    if data.archive_worker_started.swap(true, Ordering::AcqRel) {
        return;
    }
    let ctx = ctx.clone();
    let pool = data.pool.clone();
    tokio::spawn(async move { archive_worker(ctx, pool).await });
}

pub async fn enqueue_message(
    pool: &sqlx::PgPool,
    message: &sere::Message,
    is_edit: bool,
) -> Result<(), Error> {
    let payload = serde_json::to_value(ArchivePayload::Message(Box::new(message.clone())))?;
    enqueue_payload(pool, message.id.get() as i64, is_edit, payload).await
}

pub async fn enqueue_message_update(
    pool: &sqlx::PgPool,
    event: &sere::MessageUpdateEvent,
    guild_id: Option<sere::GuildId>,
) -> Result<(), Error> {
    let payload = serde_json::to_value(ArchivePayload::Update {
        event: Box::new(event.clone()),
        guild_id: guild_id.map(|id| id.get()),
    })?;
    enqueue_payload(pool, event.id.get() as i64, true, payload).await
}

async fn enqueue_payload(
    pool: &sqlx::PgPool,
    message_id: i64,
    is_edit: bool,
    payload: serde_json::Value,
) -> Result<(), Error> {
    sqlx::query(
        "INSERT INTO archive_queue (message_id, is_edit, payload)
         VALUES ($1, $2, $3)",
    )
    .bind(message_id)
    .bind(is_edit)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(())
}

async fn archive_worker(ctx: sere::Context, pool: sqlx::PgPool) {
    loop {
        match claim_next(&pool).await {
            Ok(Some((event_id, is_edit, payload))) => {
                let result = process_event(&ctx, &pool, event_id, is_edit, payload).await;
                match result {
                    Ok(()) => {
                        if let Err(error) = sqlx::query("DELETE FROM archive_queue WHERE id = $1")
                            .bind(event_id)
                            .execute(&pool)
                            .await
                        {
                            error!("Could not remove completed archive event {event_id}: {error}");
                        }
                    }
                    Err(error) => {
                        error!("Archive event {event_id} failed: {error}");
                        if let Err(update_error) =
                            mark_failed(&pool, event_id, &error.to_string()).await
                        {
                            error!("Could not reschedule archive event {event_id}: {update_error}");
                        }
                    }
                }
            }
            Ok(None) => tokio::time::sleep(Duration::from_secs(2)).await,
            Err(error) => {
                error!("Could not claim an archive event: {error}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn process_event(
    ctx: &sere::Context,
    pool: &sqlx::PgPool,
    event_id: i64,
    is_edit: bool,
    payload: serde_json::Value,
) -> Result<(), Error> {
    let message = match serde_json::from_value::<ArchivePayload>(payload)? {
        ArchivePayload::Message(message) => *message,
        ArchivePayload::Update { event, guild_id } => {
            let mut message = event.channel_id.message(ctx, event.id).await?;
            event.apply_to_message(&mut message);
            if message.guild_id.is_none() {
                message.guild_id = guild_id.map(sere::GuildId::new);
            }
            message
        }
    };
    if message.guild_id.is_none() {
        return Err(std::io::Error::other("Archived message has no server ID").into());
    }
    record_message_version(ctx, &message, pool, is_edit, event_id).await
}

async fn claim_next(
    pool: &sqlx::PgPool,
) -> Result<Option<(i64, bool, serde_json::Value)>, sqlx::Error> {
    sqlx::query_as(
        "UPDATE archive_queue
         SET attempts = attempts + 1, locked_until = NOW() + INTERVAL '5 minutes'
         WHERE id = (
             SELECT q.id FROM archive_queue q
             WHERE q.failed_at IS NULL
               AND q.available_at <= NOW()
               AND (q.locked_until IS NULL OR q.locked_until <= NOW())
               AND NOT EXISTS (
                   SELECT 1 FROM archive_queue earlier
                   WHERE earlier.message_id = q.message_id
                     AND earlier.id < q.id
                     AND earlier.failed_at IS NULL
               )
             ORDER BY q.available_at, q.id
             FOR UPDATE SKIP LOCKED
             LIMIT 1
         )
         RETURNING id, is_edit, payload",
    )
    .fetch_optional(pool)
    .await
}

async fn mark_failed(pool: &sqlx::PgPool, event_id: i64, message: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "WITH failed AS (
             UPDATE archive_queue
             SET last_error = $2,
                 locked_until = NULL,
                 available_at = NOW() + LEAST(POWER(2, attempts), 300) * INTERVAL '1 second',
                 failed_at = CASE WHEN attempts >= $3 THEN NOW() ELSE NULL END
             WHERE id = $1
             RETURNING message_id, failed_at
         )
         UPDATE messages
         SET archive_incomplete = TRUE
         FROM failed
         WHERE messages.message_id = failed.message_id
           AND failed.failed_at IS NOT NULL",
    )
    .bind(event_id)
    .bind(message)
    .bind(MAX_ARCHIVE_ATTEMPTS)
    .execute(pool)
    .await?;
    Ok(())
}

async fn record_message_version(
    ctx: &sere::Context,
    message: &sere::Message,
    pool: &sqlx::PgPool,
    is_edit: bool,
    source_event_id: i64,
) -> Result<(), Error> {
    let Some(guild_id) = message.guild_id else {
        trace!("Ignoring message {} without a server", message.id.get());
        return Ok(());
    };

    trace!(
        "Archiving message {} from channel {} in guild {}",
        message.id.get(),
        message.channel_id.get(),
        guild_id.get()
    );

    let guild = guild_id.to_partial_guild(ctx).await?;
    let channel = message
        .channel_id
        .to_channel(ctx)
        .await?
        .guild()
        .ok_or_else(|| std::io::Error::other("Guild message belonged to a private channel"))?;

    let mut downloaded_attachments = Vec::with_capacity(message.attachments.len());
    for attachment in &message.attachments {
        trace!(
            "Downloading attachment {} from message {}",
            attachment.id.get(),
            message.id.get()
        );
        downloaded_attachments.push((attachment, attachment.download().await?));
    }

    let guild_id = guild_id.get() as i64;
    let channel_id = message.channel_id.get() as i64;
    let message_id = message.id.get() as i64;
    let author_id = message.author.id.get() as i64;
    let timestamp = message.timestamp.to_string();
    let mut transaction = pool.begin().await?;

    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(message_id)
        .execute(&mut *transaction)
        .await?;

    if sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM message_versions WHERE source_event_id = $1)",
    )
    .bind(source_event_id)
    .fetch_one(&mut *transaction)
    .await?
    {
        transaction.commit().await?;
        return Ok(());
    }

    if !is_edit
        && sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM message_versions WHERE message_id = $1)",
        )
        .bind(message_id)
        .fetch_one(&mut *transaction)
        .await?
    {
        transaction.commit().await?;
        return Ok(());
    }

    sqlx::query(
        "INSERT INTO guilds (guild_id, guild_name, guild_icon_url)
         VALUES ($1, $2, $3)
         ON CONFLICT (guild_id) DO UPDATE SET
             guild_name = EXCLUDED.guild_name,
             guild_icon_url = EXCLUDED.guild_icon_url;",
    )
    .bind(guild_id)
    .bind(&guild.name)
    .bind(guild.icon_url())
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "INSERT INTO guild_history (guild_id, guild_name, guild_icon_url)
         VALUES ($1, $2, $3)
         ON CONFLICT (guild_id, guild_name, guild_icon_url) DO UPDATE SET
             last_seen_at = NOW();",
    )
    .bind(guild_id)
    .bind(&guild.name)
    .bind(guild.icon_url())
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "INSERT INTO channels (guild_id, channel_id, channel_name)
         VALUES ($1, $2, $3)
         ON CONFLICT (channel_id) DO UPDATE SET
             guild_id = EXCLUDED.guild_id,
             channel_name = EXCLUDED.channel_name;",
    )
    .bind(guild_id)
    .bind(channel_id)
    .bind(&channel.name)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "INSERT INTO channel_history (channel_id, channel_name)
         VALUES ($1, $2)
         ON CONFLICT (channel_id, channel_name) DO UPDATE SET
             last_seen_at = NOW();",
    )
    .bind(channel_id)
    .bind(&channel.name)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "INSERT INTO discord_users (discord_id, discord_username)
         VALUES ($1, $2)
         ON CONFLICT (discord_id) DO UPDATE SET
             discord_username = EXCLUDED.discord_username;",
    )
    .bind(author_id)
    .bind(&message.author.name)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "INSERT INTO discord_user_history (discord_id, discord_username, discord_avatar_url)
         VALUES ($1, $2, $3)
         ON CONFLICT (discord_id, discord_username, discord_avatar_url) DO UPDATE SET
             last_seen_at = NOW();",
    )
    .bind(author_id)
    .bind(&message.author.name)
    .bind(message.author.avatar_url())
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "INSERT INTO guild_users (guild_id, discord_id)
         VALUES ($1, $2)
         ON CONFLICT (guild_id, discord_id) DO UPDATE SET
             last_seen_at = NOW();",
    )
    .bind(guild_id)
    .bind(author_id)
    .execute(&mut *transaction)
    .await?;

    for role in guild.roles.values() {
        sqlx::query(
            "INSERT INTO discord_roles (guild_id, discord_id, discord_role_name)
             VALUES ($1, $2, $3)
             ON CONFLICT (guild_id, discord_id) DO UPDATE SET
                 discord_role_name = EXCLUDED.discord_role_name;",
        )
        .bind(guild_id)
        .bind(role.id.get() as i64)
        .bind(&role.name)
        .execute(&mut *transaction)
        .await?;
    }

    sqlx::query(
        "INSERT INTO messages (
             guild_id, channel_id, message_id, author_id, author_username, content, timestamp
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz)
         ON CONFLICT (message_id) DO UPDATE SET
             guild_id = EXCLUDED.guild_id,
             channel_id = EXCLUDED.channel_id,
             author_id = EXCLUDED.author_id,
             author_username = EXCLUDED.author_username,
             content = EXCLUDED.content,
             timestamp = EXCLUDED.timestamp;",
    )
    .bind(guild_id)
    .bind(channel_id)
    .bind(message_id)
    .bind(author_id)
    .bind(&message.author.name)
    .bind(&message.content)
    .bind(timestamp)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "UPDATE messages
         SET archive_incomplete = TRUE
         WHERE message_id = $1
           AND EXISTS(
               SELECT 1 FROM archive_queue
               WHERE message_id = $1 AND failed_at IS NOT NULL
           )",
    )
    .bind(message_id)
    .execute(&mut *transaction)
    .await?;

    sqlx::query("SELECT message_id FROM messages WHERE message_id = $1 FOR UPDATE")
        .bind(message_id)
        .fetch_one(&mut *transaction)
        .await?;

    if sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM message_versions WHERE source_event_id = $1)",
    )
    .bind(source_event_id)
    .fetch_one(&mut *transaction)
    .await?
    {
        transaction.commit().await?;
        return Ok(());
    }

    let message_version = if is_edit {
        sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(version), 0) + 1
             FROM message_versions
             WHERE message_id = $1;",
        )
        .bind(message_id)
        .fetch_one(&mut *transaction)
        .await?
    } else {
        1
    };

    sqlx::query(
        "INSERT INTO message_versions (message_id, version, content, source_event_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(message_id)
    .bind(message_version)
    .bind(&message.content)
    .bind(source_event_id)
    .execute(&mut *transaction)
    .await?;

    for (attachment, data) in downloaded_attachments {
        sqlx::query(
            "INSERT INTO attachments (
                 attachment_id, message_id, message_version, filename, description, content_type,
                 size, width, height, data
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (message_id, message_version, attachment_id) DO UPDATE SET
                 filename = EXCLUDED.filename,
                 description = EXCLUDED.description,
                 content_type = EXCLUDED.content_type,
                 size = EXCLUDED.size,
                 width = EXCLUDED.width,
                 height = EXCLUDED.height,
                 data = EXCLUDED.data;",
        )
        .bind(attachment.id.get() as i64)
        .bind(message_id)
        .bind(message_version)
        .bind(&attachment.filename)
        .bind(&attachment.description)
        .bind(&attachment.content_type)
        .bind(i64::from(attachment.size))
        .bind(optional_u32_to_i32(attachment.width)?)
        .bind(optional_u32_to_i32(attachment.height)?)
        .bind(data)
        .execute(&mut *transaction)
        .await?;
    }

    for (embed_index, embed) in message.embeds.iter().enumerate() {
        let embed_index = i32::try_from(embed_index)?;
        let embed_timestamp = embed.timestamp.map(|timestamp| timestamp.to_string());

        sqlx::query(
            "INSERT INTO embeds (
                 message_id, message_version, embed_index, embed_type, title, description, url,
                 timestamp, color,
                 footer_text, footer_icon_url, image_url, image_proxy_url, image_width,
                 image_height, thumbnail_url, thumbnail_proxy_url, thumbnail_width,
                 thumbnail_height, video_url, video_proxy_url, video_width, video_height,
                 provider_name, provider_url, author_name, author_url, author_icon_url,
                 author_proxy_icon_url
             )
             VALUES (
                 $1, $2, $3, $4, $5, $6, $7, $8::timestamptz, $9, $10, $11, $12, $13, $14,
                 $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29
             );",
        )
        .bind(message_id)
        .bind(message_version)
        .bind(embed_index)
        .bind(&embed.kind)
        .bind(&embed.title)
        .bind(&embed.description)
        .bind(&embed.url)
        .bind(embed_timestamp)
        .bind(embed.colour.map(|colour| colour.0 as i32))
        .bind(embed.footer.as_ref().map(|footer| &footer.text))
        .bind(
            embed
                .footer
                .as_ref()
                .and_then(|footer| footer.icon_url.as_ref()),
        )
        .bind(embed.image.as_ref().map(|image| &image.url))
        .bind(
            embed
                .image
                .as_ref()
                .and_then(|image| image.proxy_url.as_ref()),
        )
        .bind(optional_u32_to_i32(
            embed.image.as_ref().and_then(|image| image.width),
        )?)
        .bind(optional_u32_to_i32(
            embed.image.as_ref().and_then(|image| image.height),
        )?)
        .bind(embed.thumbnail.as_ref().map(|thumbnail| &thumbnail.url))
        .bind(
            embed
                .thumbnail
                .as_ref()
                .and_then(|thumbnail| thumbnail.proxy_url.as_ref()),
        )
        .bind(optional_u32_to_i32(
            embed
                .thumbnail
                .as_ref()
                .and_then(|thumbnail| thumbnail.width),
        )?)
        .bind(optional_u32_to_i32(
            embed
                .thumbnail
                .as_ref()
                .and_then(|thumbnail| thumbnail.height),
        )?)
        .bind(embed.video.as_ref().map(|video| &video.url))
        .bind(
            embed
                .video
                .as_ref()
                .and_then(|video| video.proxy_url.as_ref()),
        )
        .bind(optional_u32_to_i32(
            embed.video.as_ref().and_then(|video| video.width),
        )?)
        .bind(optional_u32_to_i32(
            embed.video.as_ref().and_then(|video| video.height),
        )?)
        .bind(
            embed
                .provider
                .as_ref()
                .and_then(|provider| provider.name.as_ref()),
        )
        .bind(
            embed
                .provider
                .as_ref()
                .and_then(|provider| provider.url.as_ref()),
        )
        .bind(embed.author.as_ref().map(|author| &author.name))
        .bind(embed.author.as_ref().and_then(|author| author.url.as_ref()))
        .bind(
            embed
                .author
                .as_ref()
                .and_then(|author| author.icon_url.as_ref()),
        )
        .bind(
            embed
                .author
                .as_ref()
                .and_then(|author| author.proxy_icon_url.as_ref()),
        )
        .execute(&mut *transaction)
        .await?;

        for (field_index, field) in embed.fields.iter().enumerate() {
            sqlx::query(
                "INSERT INTO embed_fields (embed_uuid, field_index, name, value, inline)
                 SELECT uuid, $4, $5, $6, $7
                 FROM embeds
                 WHERE message_id = $1 AND message_version = $2 AND embed_index = $3;",
            )
            .bind(message_id)
            .bind(message_version)
            .bind(embed_index)
            .bind(i32::try_from(field_index)?)
            .bind(&field.name)
            .bind(&field.value)
            .bind(field.inline)
            .execute(&mut *transaction)
            .await?;
        }
    }

    transaction.commit().await?;
    debug!(
        "Archived version {} of message {} with {} attachments and {} embeds",
        message_version,
        message.id.get(),
        message.attachments.len(),
        message.embeds.len()
    );

    Ok(())
}

fn optional_u32_to_i32(value: Option<u32>) -> Result<Option<i32>, std::num::TryFromIntError> {
    value.map(i32::try_from).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> Option<sqlx::PgPool> {
        let database_url = std::env::var("DATABASE_URL").ok()?;
        let pool = sqlx::PgPool::connect(&database_url).await.ok()?;
        sqlx::migrate!().run(&pool).await.ok()?;
        Some(pool)
    }

    #[tokio::test]
    async fn keeps_later_message_events_blocked_until_the_first_event_dead_letters() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let message_id = -9_000_000_001_i64;
        sqlx::query("DELETE FROM archive_queue WHERE message_id = $1")
            .bind(message_id)
            .execute(&pool)
            .await
            .unwrap();
        let first = sqlx::query_scalar::<_, i64>(
            "INSERT INTO archive_queue (message_id, is_edit, payload, attempts)
             VALUES ($1, TRUE, '{}'::jsonb, 9)
             RETURNING id",
        )
        .bind(message_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let second = sqlx::query_scalar::<_, i64>(
            "INSERT INTO archive_queue (message_id, is_edit, payload)
             VALUES ($1, TRUE, '{}'::jsonb)
             RETURNING id",
        )
        .bind(message_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(claim_next(&pool).await.unwrap().unwrap().0, first);
        assert!(claim_next(&pool).await.unwrap().is_none());
        mark_failed(&pool, first, "test").await.unwrap();
        assert_eq!(claim_next(&pool).await.unwrap().unwrap().0, second);

        sqlx::query("DELETE FROM archive_queue WHERE message_id = $1")
            .bind(message_id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
