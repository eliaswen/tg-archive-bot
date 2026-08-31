use poise::serenity_prelude as sere;
use std::env;
use tracing::{debug, error, info, trace};
use tracing_subscriber::EnvFilter;
mod archive_stats;
mod bot;
mod web;

pub struct Data {
    pub pool: sqlx::PgPool,
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Data, Error>;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    let log_level = env::var("TG_BOT_LOG").unwrap_or_else(|_| "info".to_string());
    let filter = EnvFilter::new(format!("tg_archive_bot={}", log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_file(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_line_number(true)
        .with_target(true)
        .init();

    trace!("Logging system started");
    info!("Log Level: {}", log_level);

    trace!("Env loaded");

    trace!("Loading database url");
    let database_url =
        env::var("TG_BOT_DATABASE_URL").expect("Expected a database url in the environment");
    trace!("Database url loaded");

    trace!("Loading token");
    let token = env::var("TG_BOT_DISCORD_TOKEN").expect("Expected a token in the environment");
    trace!("Token loaded");

    trace!("Loading web address");
    let web_address = env::var("TG_BOT_WEB_ADDRESS").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    trace!("Web address loaded");

    let discord_client_id = env::var("TG_BOT_DISCORD_CLIENT_ID")
        .expect("Expected a Discord client ID in the environment");
    let discord_client_secret = env::var("TG_BOT_DISCORD_CLIENT_SECRET")
        .expect("Expected a Discord client secret in the environment");
    let discord_redirect_uri = env::var("TG_BOT_DISCORD_REDIRECT_URI")
        .expect("Expected a Discord redirect URI in the environment");

    info!("Starting bot...");

    debug!("Initalizing database");
    let pool = connect_database(&database_url)
        .await
        .expect("Failed to connect to database");
    debug!("Database connected");

    trace!("Applying migrations");
    sqlx::migrate!()
        .run(&pool)
        .await
        .expect("Failed to apply database migrations");
    debug!("Migrations applied");

    debug!("Starting web server on {}", web_address);
    let web_listener = tokio::net::TcpListener::bind(&web_address)
        .await
        .expect("Failed to bind web server");
    tokio::spawn(web::run(
        web_listener,
        pool.clone(),
        token.clone(),
        discord_client_id,
        discord_client_secret,
        discord_redirect_uri,
    ));
    debug!("Web server started");

    trace!("Loading intents");
    let intents = sere::GatewayIntents::non_privileged()
        | sere::GatewayIntents::MESSAGE_CONTENT
        | sere::GatewayIntents::GUILD_MEMBERS;
    trace!("Intents loaded");

    trace!("Initalizing framework");
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            event_handler: |ctx, event, framework, user_data| {
                Box::pin(bot::event_handler::event_handler(
                    ctx, event, framework, user_data,
                ))
            },
            on_error: |error| {
                Box::pin(async move {
                    match error {
                        poise::FrameworkError::Command { error, ctx, .. } => {
                            error!("Error in command `{}`: {:?}", ctx.command().name, error);
                        }
                        error => {
                            if let Err(e) = poise::builtins::on_error(error).await {
                                error!("Error while handling error: {}", e);
                            }
                        }
                    }
                })
            },
            commands: vec![
                bot::commands::ping(),
                bot::commands::version(),
                bot::commands::git(),
                bot::commands::stats(),
            ],
            ..Default::default()
        })
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                if let Ok(guild_id) = env::var("TG_BOT_GUILD_ID") {
                    let guild_id = sere::GuildId::new(guild_id.parse()?);
                    poise::builtins::register_in_guild(
                        ctx,
                        &framework.options().commands,
                        guild_id,
                    )
                    .await?;
                } else {
                    poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                }
                Ok(Data { pool: pool.clone() })
            })
        })
        .build();
    trace!("Framework initialized");

    debug!("Connecting to Discord");
    let mut client = sere::ClientBuilder::new(&token, intents)
        .framework(framework)
        .await
        .expect("Err creating client");
    debug!("Connected to Discord");

    info!("Bot running");
    if let Err(why) = client.start().await {
        error!("Client error: {why:?}");
    }
}

async fn connect_database(database_url: &str) -> Result<sqlx::PgPool, Error> {
    let options: sqlx::postgres::PgConnectOptions = database_url.parse()?;

    match sqlx::postgres::PgPoolOptions::new()
        .connect_with(options.clone())
        .await
    {
        Ok(pool) => Ok(pool),
        Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("3D000") => {
            let database_name = options.get_database().ok_or_else(|| {
                std::io::Error::other("Database URL does not contain a database name")
            })?;
            let database_name = quote_identifier(database_name);
            info!("Creating database {}", database_name);

            let maintenance_pool = sqlx::postgres::PgPoolOptions::new()
                .connect_with(options.database("postgres"))
                .await?;
            let create_database = format!("CREATE DATABASE {database_name};");
            sqlx::query(sqlx::AssertSqlSafe(create_database))
                .execute(&maintenance_pool)
                .await?;
            maintenance_pool.close().await;

            sqlx::postgres::PgPoolOptions::new()
                .connect(database_url)
                .await
                .map_err(Into::into)
        }
        Err(error) => Err(error.into()),
    }
}

fn quote_identifier(identifier: &str) -> String {
    let mut escaped = String::with_capacity(identifier.len());
    for character in identifier.chars() {
        escaped.push(character);
        if character == '"' {
            escaped.push(character);
        }
    }
    format!("\"{}\"", escaped)
}
