//! Blocking and reporting.

use axum::{
    extract::{Path, State},
    Json,
};
use tomo_shared::{BlockRequest, Profile, ReportRequest, REPORT_REASONS};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::ApiError,
    routes::{require_profile, ProfileRow},
    AppState,
};

/// True if either user has blocked the other.
pub async fn is_blocked(db: &sqlx::PgPool, a: Uuid, b: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM blocks
                        WHERE (blocker_id = $1 AND blocked_id = $2) OR (blocker_id = $2 AND blocked_id = $1))",
    )
    .bind(a)
    .bind(b)
    .fetch_one(db)
    .await
}

async fn user_exists(db: &sqlx::PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(id)
        .fetch_one(db)
        .await
}

async fn insert_block(db: &sqlx::PgPool, blocker: Uuid, blocked: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO blocks (blocker_id, blocked_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(blocker)
        .bind(blocked)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn block(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
    Json(req): Json<BlockRequest>,
) -> Result<(), ApiError> {
    if req.user_id == uid {
        return Err(ApiError::BadRequest("You cannot block yourself.".into()));
    }
    if !user_exists(&state.db, req.user_id).await? {
        return Err(ApiError::NotFound("That player does not exist."));
    }
    insert_block(&state.db, uid, req.user_id).await?;
    Ok(())
}

pub async fn unblock(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
    Path(target): Path<Uuid>,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM blocks WHERE blocker_id = $1 AND blocked_id = $2")
        .bind(uid)
        .bind(target)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// People I have blocked (not people who blocked me).
pub async fn list_blocks(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
) -> Result<Json<Vec<Profile>>, ApiError> {
    let rows = sqlx::query_as::<_, ProfileRow>(concat!(
        "SELECT ",
        profile_cols_p!(),
        " FROM profiles p JOIN blocks b ON b.blocked_id = p.user_id
          WHERE b.blocker_id = $1 ORDER BY b.created_at DESC"
    ))
    .bind(uid)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(Profile::from).collect()))
}

pub async fn report(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
    Json(req): Json<ReportRequest>,
) -> Result<(), ApiError> {
    require_profile(&state.db, uid).await?;
    if req.user_id == uid {
        return Err(ApiError::BadRequest("You cannot report yourself.".into()));
    }
    if !REPORT_REASONS.iter().any(|(key, _)| *key == req.reason) {
        return Err(ApiError::BadRequest("Pick a reason for the report.".into()));
    }
    let details = req.details.trim();
    if details.chars().count() > 1000 {
        return Err(ApiError::BadRequest("Details must be 1000 characters or fewer.".into()));
    }
    if !user_exists(&state.db, req.user_id).await? {
        return Err(ApiError::NotFound("That player does not exist."));
    }

    let inserted = sqlx::query(
        "INSERT INTO reports (reporter_id, reported_id, reason, details) VALUES ($1, $2, $3, $4)
         ON CONFLICT (reporter_id, reported_id) WHERE status = 'open' DO NOTHING",
    )
    .bind(uid)
    .bind(req.user_id)
    .bind(&req.reason)
    .bind(details)
    .execute(&state.db)
    .await?
    .rows_affected();

    // Blocking is independent of whether the report was a duplicate.
    if req.also_block {
        insert_block(&state.db, uid, req.user_id).await?;
    }
    if inserted == 0 {
        return Err(ApiError::Conflict("You already have an open report for this player."));
    }
    Ok(())
}
