use crate::messaging::trace_message;
use crate::{Context, Error};
use tracing::{debug, trace};

const VERSION: &str = include_str!(concat!(env!("OUT_DIR"), "/version.txt"));

/// Get the bot's current version
#[poise::command(slash_command, prefix_command)]
pub async fn version(ctx: Context<'_>) -> Result<(), Error> {
    trace!(
        "version command called by user {} in guild {}",
        ctx.author().id.get(),
        ctx.guild_id().unwrap().get()
    );
    trace!("Loading embedded version information");
    let msg = format!("Current version:\n{VERSION}");
    trace_message(
        &msg,
        ctx.channel_id().to_string(),
        ctx.guild_id().unwrap().to_string(),
    )
    .await;
    ctx.say(msg).await?;
    debug!(
        "Version command performed for user {} with version information",
        ctx.author().name
    );
    Ok(())
}
