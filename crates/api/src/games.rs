//! The Steam game picker: a "popular right now" shelf and name search.
//!
//! Everything the server returns is remembered in the `games` table. A profile may only list games
//! found in that table, so a game's name always comes from Steam and never from a client.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use tomo_shared::Game;

use crate::{auth::AuthUser, error::ApiError, steam::SteamError, AppState};

const SEARCH_MIN_CHARS: usize = 2;
const SEARCH_MAX_CHARS: usize = 60;

impl From<SteamError> for ApiError {
    fn from(_: SteamError) -> Self {
        ApiError::Unavailable("Steam is not responding right now. Please try again in a moment.")
    }
}

/// Remember these games so profiles can reference them.
async fn remember(db: &sqlx::PgPool, games: &[Game]) -> Result<(), sqlx::Error> {
    if games.is_empty() {
        return Ok(());
    }
    let ids: Vec<i32> = games.iter().map(|g| g.app_id as i32).collect();
    let names: Vec<String> = games.iter().map(|g| g.name.clone()).collect();
    let banners: Vec<Option<String>> = games.iter().map(|g| g.banner.clone()).collect();
    sqlx::query(
        "INSERT INTO games (app_id, name, banner) SELECT * FROM UNNEST($1::int4[], $2::text[], $3::text[])
         ON CONFLICT (app_id) DO UPDATE
            SET name = EXCLUDED.name, banner = COALESCE(EXCLUDED.banner, games.banner), updated_at = now()",
    )
    .bind(ids)
    .bind(names)
    .bind(banners)
    .execute(db)
    .await?;
    Ok(())
}

/// True when every id is a game we have seen from Steam. Used when saving a profile.
pub async fn all_known(db: &sqlx::PgPool, ids: &[i32]) -> Result<bool, sqlx::Error> {
    let found = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM games WHERE app_id = ANY($1)")
        .bind(ids)
        .fetch_one(db)
        .await?;
    Ok(found as usize == ids.len())
}

/// Most-played games on Steam right now.
pub async fn popular(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
) -> Result<Json<Vec<Game>>, ApiError> {
    let games = state.steam.popular().await?;
    remember(&state.db, &games).await?;
    Ok(Json(games))
}

#[derive(Deserialize)]
pub struct SearchParams {
    q: Option<String>,
}

/// Search Steam by name. Needs a sign-in, so this server is not an open proxy to Steam.
pub async fn search(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<Game>>, ApiError> {
    let q = params.q.unwrap_or_default();
    let q = q.trim();
    // Typing the first letter is not a search yet.
    if q.chars().count() < SEARCH_MIN_CHARS {
        return Ok(Json(vec![]));
    }
    if q.chars().count() > SEARCH_MAX_CHARS {
        return Err(ApiError::BadRequest(format!(
            "Search for at most {SEARCH_MAX_CHARS} characters."
        )));
    }
    let games = state.steam.search(q).await?;
    remember(&state.db, &games).await?;
    Ok(Json(games))
}
