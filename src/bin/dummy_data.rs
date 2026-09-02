use chrono::{Duration, Utc};
use rand::{RngExt, seq::IndexedRandom};
use sha2::{Digest, Sha256};
use std::env;

const GUILD_COUNT: i64 = 3;
const CHANNELS_PER_GUILD: i64 = 10;
const USER_COUNT: i64 = 36;
const MESSAGE_COUNT: i64 = 600;
const ZAP_USER_ID: i64 = 9_000_000_000_000_000_001;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    let database_url =
        env::var("TG_BOT_DATABASE_URL").expect("Expected a database url in the environment");
    let pool = sqlx::PgPool::connect(&database_url).await?;
    sqlx::migrate!().run(&pool).await?;

    let mut rng = rand::rng();
    let id_base = rng.random_range(1_000_000_000_000_000_000i64..4_000_000_000_000_000_000i64);
    let mut transaction = pool.begin().await?;

    let guild_names = ["Northstar Assembly", "Civic Commons", "The Policy Lab"];
    let channel_names = [
        "general",
        "announcements",
        "introductions",
        "debates",
        "legislation",
        "elections",
        "committees",
        "events",
        "resources",
        "off-topic",
    ];
    let usernames = [
        "amber", "atlas", "bailey", "beacon", "cedar", "clover", "cosmo", "dahlia", "delta",
        "echo", "ember", "finch", "flora", "forest", "harbor", "hazel", "indigo", "ivy", "juno",
        "kestrel", "lark", "linden", "maple", "marina", "milo", "nova", "olive", "onyx", "orion",
        "pearl", "quill", "river", "robin", "sage", "sol", "wren",
    ];
    let words = [
        "agenda",
        "amendment",
        "archive",
        "ballot",
        "budget",
        "candidate",
        "caucus",
        "community",
        "committee",
        "consensus",
        "constitution",
        "debate",
        "delegate",
        "discussion",
        "election",
        "event",
        "feedback",
        "forum",
        "idea",
        "meeting",
        "motion",
        "policy",
        "proposal",
        "question",
        "report",
        "resolution",
        "schedule",
        "session",
        "speech",
        "update",
        "vote",
        "workshop",
    ];

    for guild_index in 0..GUILD_COUNT {
        let guild_id = id_base + guild_index;
        let guild_name = guild_names[guild_index as usize];

        sqlx::query(
            "INSERT INTO guilds (guild_id, guild_name, guild_icon_url) VALUES ($1, $2, NULL)",
        )
        .bind(guild_id)
        .bind(guild_name)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO guild_history (guild_id, guild_name, guild_icon_url) VALUES ($1, $2, NULL)",
        )
        .bind(guild_id)
        .bind(guild_name)
        .execute(&mut *transaction)
        .await?;

        for channel_index in 0..CHANNELS_PER_GUILD {
            let channel_id = id_base + 1_000 + guild_index * CHANNELS_PER_GUILD + channel_index;
            let channel_name = channel_names[channel_index as usize];

            sqlx::query(
                "INSERT INTO channels (guild_id, channel_id, channel_name) VALUES ($1, $2, $3)",
            )
            .bind(guild_id)
            .bind(channel_id)
            .bind(channel_name)
            .execute(&mut *transaction)
            .await?;
            sqlx::query("INSERT INTO channel_history (channel_id, channel_name) VALUES ($1, $2)")
                .bind(channel_id)
                .bind(channel_name)
                .execute(&mut *transaction)
                .await?;
        }

        for role_index in 0..4i64 {
            sqlx::query(
                "INSERT INTO discord_roles (guild_id, discord_id, discord_role_name) VALUES ($1, $2, $3)",
            )
            .bind(guild_id)
            .bind(id_base + 2_000 + guild_index * 4 + role_index)
            .bind(["Member", "Moderator", "Representative", "Organizer"][role_index as usize])
            .execute(&mut *transaction)
            .await?;
        }
    }

    for user_index in 0..USER_COUNT {
        let user_id = if user_index == 0 {
            ZAP_USER_ID
        } else {
            id_base + 3_000 + user_index
        };
        let username = if user_index == 0 {
            "zap"
        } else {
            usernames[user_index as usize]
        };

        sqlx::query("INSERT INTO discord_users (discord_id, discord_username) VALUES ($1, $2)")
            .bind(user_id)
            .bind(username)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO discord_user_history (discord_id, discord_username, discord_avatar_url) VALUES ($1, $2, NULL)",
        )
        .bind(user_id)
        .bind(username)
        .execute(&mut *transaction)
        .await?;
    }

    if let Ok(token) = env::var("TG_BOT_ZAP_API_TOKEN") {
        sqlx::query("INSERT INTO api_tokens (token_hash, discord_id) VALUES ($1, $2)")
            .bind(Sha256::digest(token.as_bytes()).to_vec())
            .bind(ZAP_USER_ID)
            .execute(&mut *transaction)
            .await?;
    }

    for message_index in 0..MESSAGE_COUNT {
        let guild_index = rng.random_range(0..GUILD_COUNT);
        let channel_index = rng.random_range(0..CHANNELS_PER_GUILD);
        let user_index = rng.random_range(0..USER_COUNT);
        let guild_id = id_base + guild_index;
        let channel_id = id_base + 1_000 + guild_index * CHANNELS_PER_GUILD + channel_index;
        let user_id = if user_index == 0 {
            ZAP_USER_ID
        } else {
            id_base + 3_000 + user_index
        };
        let message_id = id_base + 4_000 + message_index;
        let username = if user_index == 0 {
            "zap"
        } else {
            usernames[user_index as usize]
        };
        let word_count = rng.random_range(3..24);
        let content = (0..word_count)
            .map(|_| *words.choose(&mut rng).unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        let timestamp =
            (Utc::now() - Duration::seconds(rng.random_range(0..15_552_000))).to_rfc3339();

        sqlx::query(
            "INSERT INTO guild_users (guild_id, discord_id) VALUES ($1, $2)
             ON CONFLICT (guild_id, discord_id) DO UPDATE SET last_seen_at = NOW()",
        )
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO messages (
                 guild_id, channel_id, message_id, author_id, author_username, content, timestamp
             ) VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz)",
        )
        .bind(guild_id)
        .bind(channel_id)
        .bind(message_id)
        .bind(user_id)
        .bind(username)
        .bind(&content)
        .bind(timestamp)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO message_versions (message_id, version, content) VALUES ($1, 1, $2)",
        )
        .bind(message_id)
        .bind(content)
        .execute(&mut *transaction)
        .await?;
    }

    transaction.commit().await?;
    println!(
        "Added {GUILD_COUNT} servers, {} channels, {USER_COUNT} users, and {MESSAGE_COUNT} messages",
        GUILD_COUNT * CHANNELS_PER_GUILD
    );

    Ok(())
}
