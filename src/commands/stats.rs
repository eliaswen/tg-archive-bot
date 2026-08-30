use crate::messaging::trace_message;
use crate::{Context, Error, web};
use tracing::{debug, trace};

#[doc = "Show archive statistics"]
#[poise::command(slash_command, prefix_command)]
pub async fn stats(ctx: Context<'_>) -> Result<(), Error> {
    trace!(
        "stats command called by user {} in guild {}",
        ctx.author().id.get(),
        ctx.guild_id().unwrap().get()
    );
    let stats = web::archive_stats(&ctx.data().pool).await?;
    let msg = format!(
        "# Archive Statistics\n## Messages\nArchived messages: {}\nArchived users: {}\nChannels: {}\nServers: {}\n## Storage usage\nArchived data: {}\nMessage content: {}\nAttachments: {}",
        stats.messages,
        stats.users,
        stats.channels,
        stats.servers,
        web::format_bytes(stats.total_storage),
        web::format_bytes(stats.message_storage),
        web::format_bytes(stats.attachment_storage),
    );
    trace_message(
        &msg,
        ctx.channel_id().to_string(),
        ctx.guild_id().unwrap().to_string(),
    )
    .await;
    ctx.say(msg).await?;
    debug!("Stats command performed for user {}", ctx.author().name);
    Ok(())
}
