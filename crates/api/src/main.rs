// SQL column lists as macros (not consts) so queries can be assembled with `concat!` into
// a `&'static str`. sqlx rejects runtime-built SQL, which keeps injection out by construction.
// Macros must be defined before the `mod` lines that use them.
//
// `game_ids` / `game_names` are built from the stored Steam ids joined to the `games` table, so
// both arrays always line up and the names always come from Steam.
macro_rules! profile_cols {
    () => {
        "user_id, display_name, bio, avatar, avatar_url, hue, rank, region, playstyle, mic,
         ARRAY(SELECT g.app_id FROM unnest(game_ids) WITH ORDINALITY AS u(id, ord)
               JOIN games g ON g.app_id = u.id ORDER BY u.ord) AS game_ids,
         ARRAY(SELECT g.name FROM unnest(game_ids) WITH ORDINALITY AS u(id, ord)
               JOIN games g ON g.app_id = u.id ORDER BY u.ord) AS game_names,
         ARRAY(SELECT g.banner FROM unnest(game_ids) WITH ORDINALITY AS u(id, ord)
               JOIN games g ON g.app_id = u.id ORDER BY u.ord) AS game_banners"
    };
}
macro_rules! profile_cols_p {
    () => {
        "p.user_id, p.display_name, p.bio, p.avatar, p.avatar_url, p.hue, p.rank, p.region, p.playstyle, p.mic,
         ARRAY(SELECT g.app_id FROM unnest(p.game_ids) WITH ORDINALITY AS u(id, ord)
               JOIN games g ON g.app_id = u.id ORDER BY u.ord) AS game_ids,
         ARRAY(SELECT g.name FROM unnest(p.game_ids) WITH ORDINALITY AS u(id, ord)
               JOIN games g ON g.app_id = u.id ORDER BY u.ord) AS game_names,
         ARRAY(SELECT g.banner FROM unnest(p.game_ids) WITH ORDINALITY AS u(id, ord)
               JOIN games g ON g.app_id = u.id ORDER BY u.ord) AS game_banners"
    };
}

mod auth;
mod chat;
mod config;
mod discord;
mod error;
mod friends;
mod games;
mod hub;
mod oauth;
mod ranking;
mod routes;
mod safety;
mod servers;
mod steam;

use std::{sync::Arc, time::Duration};

use axum::{
    http::{header, Method},
    routing::{delete, get, post, put},
    Router,
};
use config::Config;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    /// For calling Discord. Has timeouts, so a slow Discord can't hang a request forever.
    pub http: reqwest::Client,
    /// Steam's public store, with an in-memory cache.
    pub steam: Arc<steam::SteamClient>,
    /// Live WebSocket connections, for pushing chat events.
    pub hub: Arc<hub::Hub>,
}

fn build_router(state: AppState) -> Router {
    // Only the configured site(s) may call the API from a browser.
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(state.config.allowed_origins.clone()))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    Router::new()
        .route("/health", get(routes::health))
        .route("/auth/config", get(oauth::auth_config))
        .route("/auth/discord", get(oauth::discord_start))
        .route("/auth/discord/callback", get(oauth::discord_callback))
        .route("/auth/exchange", post(oauth::exchange))
        .route("/auth/dev", post(routes::dev_login))
        .route("/auth/logout", post(routes::logout))
        .route("/me", get(routes::me))
        .route("/me/profile", put(routes::put_profile))
        .route("/me/invite", get(friends::my_invite))
        .route(
            "/me/servers",
            get(servers::status).delete(servers::disconnect),
        )
        .route("/games/popular", get(games::popular))
        .route("/games/other", get(games::other))
        .route("/games/search", get(games::search))
        .route("/invites/{code}", get(friends::preview))
        .route("/invites/accept", post(friends::accept))
        .route("/friends", get(friends::list))
        .route("/friends/{user_id}", delete(friends::remove))
        .route("/servers/people", get(servers::people))
        .route("/discover", get(routes::discover))
        .route("/swipe", post(routes::swipe))
        .route("/matches", get(chat::matches))
        .route("/matches/{id}/messages", get(chat::history))
        .route("/ws", get(chat::ws_handler))
        .route("/blocks", get(safety::list_blocks).post(safety::block))
        .route("/blocks/{user_id}", delete(safety::unblock))
        .route("/reports", post(safety::report))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Local dev: settings come from .env.local (git-ignored). Real environment variables
    // always win over the file, which is how production is configured.
    dotenvy::from_filename(".env.local").ok();
    tracing_subscriber::fmt::init();

    let config = Config::from_env()?;
    if config.dev_auth {
        tracing::warn!(
            "DEV_AUTH is enabled: anyone can sign in as any username. Never use in production."
        );
    }
    if config.discord.is_none() {
        tracing::warn!(
            "Discord sign-in is not configured (set DISCORD_CLIENT_ID and DISCORD_CLIENT_SECRET)."
        );
    }
    tracing::info!("allowing browser origins: {:?}", config.allowed_origins);

    // Neon closes idle connections and scales to zero, so recycle ours before it does and
    // allow time for a cold start instead of failing fast.
    let db = PgPoolOptions::new()
        .max_connections(5)
        .idle_timeout(Duration::from_secs(120))
        .acquire_timeout(Duration::from_secs(15))
        .connect(&config.database_url)
        .await?;
    sqlx::migrate!("../../migrations").run(&db).await?;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let steam = Arc::new(steam::SteamClient::new(
        http.clone(),
        config.steam_store_base.clone(),
        config.steam_api_base.clone(),
    ));
    let port = config.port.clone();
    let app = build_router(AppState {
        db,
        config: Arc::new(config),
        http,
        steam,
        hub: Arc::new(hub::Hub::default()),
    });

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!("tomo-api listening on {port}");
    axum::serve(listener, app).await?;
    Ok(())
}
