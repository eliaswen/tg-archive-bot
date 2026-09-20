use crate::bot::messaging::{trace_message, edit_response_message};
use crate::{Context, Error, archive_stats};
use tracing::{debug, trace};

#[doc = "Show archive statistics"]
#[poise::command(slash_command, prefix_command)]
pub async fn stats(ctx: Context<'_>) -> Result<(), Error> {
    trace!(
        "stats command called by user {} in guild {}",
        ctx.author().id.get(),
        ctx.guild_id().unwrap().get()
    );
    let response = ctx.say("Processing...").await?;

    let stats = archive_stats::load(&ctx.data().pool).await?;
    let msg = format!(
        "# Archive Statistics\n## Messages\nArchived messages: {}\nArchived users: {}\nChannels: {}\nServers: {}\n## Machine learning\nProcessed images: {}\nUnprocessed images: {}\nAverage processing speed: {}\n## Storage usage\nArchived data: {}\nMessage content: {}\nAttachments: {}\nAverage data archived per day: {}\n## Runtime\nArchive running for: {}",
        stats.messages,
        stats.users,
        stats.channels,
        stats.servers,
        stats.processed_images,
        stats.unprocessed_images,
        archive_stats::format_processing_speed(stats.average_image_processing_ms),
        archive_stats::format_bytes(stats.total_storage),
        archive_stats::format_bytes(stats.message_storage),
        archive_stats::format_bytes(stats.attachment_storage),
        archive_stats::format_bytes(stats.average_daily_storage),
        archive_stats::format_duration(stats.running_seconds),
    );

    edit_response_message(&response, ctx, &msg, true).await?;
    trace_message(
        &msg,
        ctx.channel_id().to_string(),
        ctx.guild_id().unwrap().to_string(),
    )
    .await;
    debug!("Stats command performed for user {}", ctx.author().name);
    Ok(())
}
