// SQL column lists as macros (not consts) so queries can be assembled with `concat!` into
// a `&'static str`. sqlx rejects runtime-built SQL, which keeps injection out by construction.
// Macros must be defined before the `mod` lines that use them.
macro_rules! profile_cols {
    () => {
        "user_id, display_name, bio, avatar, hue, rank, region, playstyle, mic, games"
    };
}
macro_rules! profile_cols_p {
    () => {
        "p.user_id, p.display_name, p.bio, p.avatar, p.hue, p.rank, p.region, p.playstyle, p.mic, p.games"
    };
}

mod auth;
mod chat;
mod error;
mod hub;
mod ranking;
mod routes;
mod safety;

use std::sync::Arc;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// DEV ONLY username login. Off unless DEV_AUTH=true.
    pub dev_auth: bool,
    /// Live WebSocket connections, for pushing chat events.
    pub hub: Arc<hub::Hub>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Local dev: credentials come from .env.local (git-ignored). In production, real env vars.
    dotenvy::from_filename(".env.local").ok();
    tracing_subscriber::fmt::init();

    let database_url = std::env::var("DATABASE_URL")?;
    let dev_auth = std::env::var("DEV_AUTH").map(|v| v == "true").unwrap_or(false);
    if dev_auth {
        tracing::warn!("DEV_AUTH is enabled: anyone can sign in as any username. Never use in production.");
    }

    // Neon closes idle connections and scales to zero, so recycle ours before it does and
    // allow time for a cold start instead of failing fast.
    let db = PgPoolOptions::new()
        .max_connections(5)
        .idle_timeout(std::time::Duration::from_secs(120))
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect(&database_url)
        .await?;
    sqlx::migrate!("../../migrations").run(&db).await?;

    let app = Router::new()
        .route("/health", get(routes::health))
        .route("/auth/dev", post(routes::dev_login))
        .route("/auth/logout", post(routes::logout))
        .route("/me", get(routes::me))
        .route("/me/profile", put(routes::put_profile))
        .route("/discover", get(routes::discover))
        .route("/swipe", post(routes::swipe))
        .route("/matches", get(chat::matches))
        .route("/matches/{id}/messages", get(chat::history))
        .route("/ws", get(chat::ws_handler))
        .route("/blocks", get(safety::list_blocks).post(safety::block))
        .route("/blocks/{user_id}", delete(safety::unblock))
        .route("/reports", post(safety::report))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(AppState { db, dev_auth, hub: Arc::new(hub::Hub::default()) });

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!("tomo-api listening on {port}");
    axum::serve(listener, app).await?;
    Ok(())
}
