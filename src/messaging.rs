use crate::{Context, Error};
use tracing::trace;

pub async fn trace_message(msg: &str, channel: String, guild: String) {
    trace!(
        "Saying \"{}\" in channel {} in guild {}",
        msg, channel, guild
    );
}

pub async fn edit_response_message<'a>(
    response_message: &poise::ReplyHandle<'a>,
    ctx: Context<'_>,
    content: &str,
    silent: bool,
) -> Result<(), Error> {
    if silent {
        response_message
            .edit(
                ctx,
                poise::CreateReply::default()
                    .content(content)
                    .allowed_mentions(
                        poise::serenity_prelude::CreateAllowedMentions::new().empty_users(),
                    ),
            )
            .await?;
    } else {
        response_message
            .edit(ctx, poise::CreateReply::default().content(content))
            .await?;
    }
    Ok(())
}
