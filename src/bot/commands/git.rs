use crate::bot::messaging::{edit_response_message, trace_message};
use crate::{Context, Error};
use poise::serenity_prelude::model::timestamp;
use tracing::debug;

/// Get a link to the git repo
#[poise::command(slash_command, prefix_command)]
pub async fn git(ctx: Context<'_>) -> Result<(), Error> {
    let start_time = std::time::Instant::now();
    debug!(
        "git command ran by {} in {}",
        ctx.author().name,
        ctx.channel_id().get()
    );



    let response = ctx.say("Processing...").await?;

    let msg = "Git repo: https://github.com/eliaswen/tg-archive-bot\nIf you'd like to contribute, contact <@1389325880853270569> to get an account.";
    edit_response_message(&response, ctx, msg, true).await?;
    trace_message(
        &msg,
        ctx.channel_id().get().to_string(),
        ctx.guild_id().unwrap().get().to_string(),
    )
    .await;
    let time_taken = start_time.elapsed();
    let msg = format!(
        "{}\n*Processed command in {:?}*",
        &msg,
        time_taken
    );
    edit_response_message(&response, ctx, &msg, true).await?;

    Ok(())
}
