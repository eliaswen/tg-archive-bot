use std::os::unix::raw::time_t;

use crate::bot::messaging::{edit_response_message, trace_message};
use crate::{Context, Error};
use axum::http::response;
use poise::serenity_prelude as sere;
use tracing::{debug, trace};

/// Get the bot's current ping (back and forth)
#[poise::command(slash_command, prefix_command)]
pub async fn ping(ctx: Context<'_>) -> Result<(), Error> {
    let start_time = std::time::Instant::now();
    trace!(
        "ping command called by user {} in guild {}",
        ctx.author().id.get(),
        ctx.guild_id().unwrap().get()
    );
    let response = ctx.say("Processing...").await?;
    trace!("Calculating ping");
    let command_time_in_microseconds = ctx.created_at().timestamp_micros();
    trace!("Saved command time in microseconds");
    let current_time_in_microseconds = sere::Timestamp::now().timestamp_micros();
    trace!("Saved current time in microseconds");
    let ping = (current_time_in_microseconds - command_time_in_microseconds) / 500;
    trace!("Calculated ping: {}", ping);
    // We divide by 1000 since we get the time in microseconds, and then multiply by 2 to get the roundtrip time
    // This is arguably not the best way to calculate ping, since it assumes perfect clock accuracy, but I'm lazy
    let msg = format!("Current ping: {} ms", ping);
    edit_response_message(&response, ctx, &msg, true).await?;
    trace_message(
        &msg,
        ctx.channel_id().to_string(),
        ctx.guild_id().unwrap().to_string(),
    )
    .await;
    
    debug!(
        "Ping command performed for user {} with ping {}",
        ctx.author().name,
        ping
    );
    let time_taken = start_time.elapsed();
    let msg = format!(
        "{}\n*Processed command in {:?}*",
        &msg,
        time_taken
    );
    edit_response_message(&response, ctx, &msg, true).await?;
    Ok(())
}
