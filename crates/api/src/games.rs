//! The Steam game picker: a "popular right now" shelf and name search.
//!
//! Everything the server returns is remembered in the `games` table. A profile may only list games
//! found in that table, so a game's name always comes from Steam and never from a client.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use tomo_shared::{Game, OTHER_GAME_BASE};

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
    // Steam never returns ids in the reserved range; ignore any that do, so Steam can never
    // overwrite one of the hand-picked games.
    let games: Vec<&Game> = games
        .iter()
        .filter(|g| g.app_id < OTHER_GAME_BASE)
        .collect();
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

/// The hand-picked games that are not on Steam, optionally only those whose name contains `q`.
async fn curated(db: &sqlx::PgPool, q: Option<&str>) -> Result<Vec<Game>, sqlx::Error> {
    // `position` is a plain substring test, so `%` and `_` typed by a person are just characters.
    let rows = sqlx::query_as::<_, (i32, String)>(
        "SELECT app_id, name FROM games
         WHERE app_id >= $1 AND ($2::text IS NULL OR position(lower($2) in lower(name)) > 0)
         ORDER BY name",
    )
    .bind(OTHER_GAME_BASE as i32)
    .bind(q)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name)| Game {
            app_id: id as u32,
            name,
            banner: None,
        })
        .collect())
}

/// True when every id is a game we have seen (from Steam, or the hand-picked list). Used when saving a profile.
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

/// Hand-picked popular games that are not on Steam (Valorant, League of Legends, ...).
pub async fn other(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
) -> Result<Json<Vec<Game>>, ApiError> {
    Ok(Json(curated(&state.db, None).await?))
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
    // Hand-picked games first. If Steam is down, those alone are still a useful answer.
    let mut games = curated(&state.db, Some(q)).await?;
    match state.steam.search(q).await {
        Ok(steam) => {
            remember(&state.db, &steam).await?;
            games.extend(steam);
        }
        Err(e) if games.is_empty() => return Err(e.into()),
        Err(_) => {}
    }
    Ok(Json(games))
}
