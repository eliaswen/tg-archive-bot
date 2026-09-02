use crate::{Data, Error};
use poise::serenity_prelude as sere;
use tracing::{debug, trace};

pub async fn event_handler(
    ctx: &sere::Context,
    event: &sere::FullEvent,
    _framework: poise::FrameworkContext<'_, Data, Error>,
    user_data: &Data,
) -> Result<(), Error> {
    super::message_archive::ensure_worker_started(ctx, user_data);
    match event {
        sere::FullEvent::Message { new_message } => {
            super::message_archive::enqueue_message(&user_data.pool, new_message, false).await?;
        }
        sere::FullEvent::MessageUpdate {
            old_if_available,
            new,
            event,
        } => {
            let old_guild_id = old_if_available
                .as_ref()
                .and_then(|message| message.guild_id);
            let mut message = if let Some(new_message) = new {
                new_message.clone()
            } else if let Some(old_message) = old_if_available {
                let mut updated_message = old_message.clone();
                event.apply_to_message(&mut updated_message);
                updated_message
            } else {
                debug!(
                    "Updated message {} was not cached, queueing it for retrieval",
                    event.id.get()
                );
                super::message_archive::enqueue_message_update(
                    &user_data.pool,
                    event,
                    event.guild_id,
                )
                .await?;
                return Ok(());
            };
            message.guild_id =
                updated_message_guild_id(message.guild_id, event.guild_id, old_guild_id);
            if message.guild_id.is_none() {
                debug!(
                    "Updated message {} had no server ID, queueing it for retrieval",
                    event.id.get()
                );
                super::message_archive::enqueue_message_update(
                    &user_data.pool,
                    event,
                    event.guild_id.or(old_guild_id),
                )
                .await?;
                return Ok(());
            }

            super::message_archive::enqueue_message(&user_data.pool, &message, true).await?;
        }
        sere::FullEvent::MessageDelete {
            deleted_message_id, ..
        } => {
            trace!(
                "Message {} was deleted from Discord, retaining archived copy",
                deleted_message_id.get()
            );
        }
        sere::FullEvent::MessageDeleteBulk {
            multiple_deleted_messages_ids,
            ..
        } => {
            trace!(
                "{} messages were deleted from Discord, retaining archived copies",
                multiple_deleted_messages_ids.len()
            );
        }
        _ => {}
    }

    Ok(())
}

fn updated_message_guild_id(
    message_guild_id: Option<sere::GuildId>,
    event_guild_id: Option<sere::GuildId>,
    old_guild_id: Option<sere::GuildId>,
) -> Option<sere::GuildId> {
    message_guild_id.or(event_guild_id).or(old_guild_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_the_cached_server_when_an_update_omits_it() {
        let guild_id = sere::GuildId::new(123);

        assert_eq!(
            updated_message_guild_id(None, None, Some(guild_id)),
            Some(guild_id)
        );
    }

    #[test]
    fn prefers_the_updated_message_server() {
        let message_guild_id = sere::GuildId::new(123);
        let old_guild_id = sere::GuildId::new(456);

        assert_eq!(
            updated_message_guild_id(Some(message_guild_id), None, Some(old_guild_id)),
            Some(message_guild_id)
        );
    }
}
