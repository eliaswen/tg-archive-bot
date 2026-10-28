use super::*;

pub(in crate::web) async fn message(
    State(data): State<WebData>,
    Extension(user): Extension<WebUser>,
    Path(message_id): Path<i64>,
    Query(query): Query<MessageQuery>,
) -> WebResult {
    let message = sqlx::query_as::<_, (String, i64, i64, String, i64, String, String, bool)>(
        "SELECT
             m.author_username,
             m.author_id,
             m.guild_id,
             g.guild_name,
             m.channel_id,
             c.channel_name,
             m.timestamp::text,
             m.archive_incomplete
         FROM messages m
         JOIN guilds g ON g.guild_id = m.guild_id
         JOIN channels c ON c.channel_id = m.channel_id
         WHERE m.message_id = $1;",
    )
    .bind(message_id)
    .fetch_optional(&data.pool)
    .await
    .map_err(database_error)?;
    let Some((author, author_id, server_id, server, channel_id, channel, timestamp, archive_incomplete)) = message
    else {
        return Err(not_found("Message not found."));
    };
    let timestamp = localize_timestamp(&timestamp);

    let version_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM message_versions WHERE message_id = $1;",
    )
    .bind(message_id)
    .fetch_one(&data.pool)
    .await
    .map_err(database_error)?;
    let version = sqlx::query_as::<_, (i64, Option<String>, String)>(
        "SELECT version, content, archived_at::text
         FROM message_versions
         WHERE message_id = $1 AND ($2::bigint IS NULL OR version = $2)
         ORDER BY version DESC
         LIMIT 1;",
    )
    .bind(message_id)
    .bind(query.version)
    .fetch_optional(&data.pool)
    .await
    .map_err(database_error)?
    .ok_or_else(|| not_found("Message version not found."))?;
    let (message_version, content, archived_at) = version;

    let attachments = sqlx::query_as::<_, (i64, String, Option<String>, Option<String>, i64)>(
        "SELECT attachment_id, filename, description, content_type, size
         FROM attachments
         WHERE message_id = $1 AND message_version = $2
         ORDER BY attachment_id;",
    )
    .bind(message_id)
    .bind(message_version)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;
    let embeds = sqlx::query_as::<_, (i32, Option<String>, Option<String>, Option<String>)>(
        "SELECT embed_index, title, description, url
         FROM embeds
         WHERE message_id = $1 AND message_version = $2
         ORDER BY embed_index;",
    )
    .bind(message_id)
    .bind(message_version)
    .fetch_all(&data.pool)
    .await
    .map_err(database_error)?;

    let mut rendered_attachments = String::new();
    for (attachment_id, filename, description, content_type, size) in attachments {
        rendered_attachments.push_str(&render_template(&AttachmentTemplate {
            attachment_id,
            message_version,
            filename: &filename,
            size,
            content_type: content_type.as_deref(),
            description: description.as_deref(),
        }));
    }

    let mut rendered_embeds = String::new();
    for (_embed_index, title, description, url) in embeds {
        rendered_embeds.push_str(&render_template(&EmbedTemplate {
            title: title.as_deref(),
            description: description.as_deref(),
            url: url.as_deref(),
        }));
    }

    let body = render_template(&MessageTemplate {
        message_id,
        author: &author,
        author_id,
        server_id,
        server: &server,
        channel_id,
        channel: &channel,
        timestamp: &timestamp,
        archive_incomplete,
        version_navigation: message_version_navigation(
            message_id,
            message_version,
            version_count,
            &archived_at,
        ),
        content: content.as_deref().unwrap_or(""),
        attachments: rendered_attachments,
        embeds: rendered_embeds,
        access_reason: if author_id == user.id && !user.channel_ids.contains(&channel_id) {
            "you sent it"
        } else {
            user.access_reason(channel_id)
        },
    });
    Ok(Html(page(&format!("Message {}", message_id), &body)))
}
