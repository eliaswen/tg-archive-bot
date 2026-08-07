use crate::Error;
use poise::serenity_prelude as sere;
use tracing::{debug, trace};

pub async fn record_message(
    ctx: &sere::Context,
    message: &sere::Message,
    pool: &sqlx::PgPool,
) -> Result<(), Error> {
    record_message_version(ctx, message, pool, false).await
}

pub async fn record_message_edit(
    ctx: &sere::Context,
    message: &sere::Message,
    pool: &sqlx::PgPool,
) -> Result<(), Error> {
    record_message_version(ctx, message, pool, true).await
}

async fn record_message_version(
    ctx: &sere::Context,
    message: &sere::Message,
    pool: &sqlx::PgPool,
    is_edit: bool,
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
        "INSERT INTO message_versions (message_id, version, content)
         VALUES ($1, $2, $3)
         ON CONFLICT (message_id, version) DO UPDATE SET
             content = EXCLUDED.content;",
    )
    .bind(message_id)
    .bind(message_version)
    .bind(&message.content)
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
