//! "Players from your Discord servers": strictly opt-in.
//!
//! People choose this on the login page (which is the only time we ask Discord for their server list).
//! We keep only small communities, share only the names of servers that BOTH people are in, and
//! delete everything when someone disconnects.

use axum::{extract::State, Json};
use sqlx::FromRow;
use tomo_shared::{ServerPerson, ServersStatus};

use crate::{auth::AuthUser, discord::Guild, error::ApiError, routes::ProfileRow, AppState};

/// Replace this person's stored servers with `guilds` and mark them as opted in.
pub async fn store_guilds(
    db: &sqlx::PgPool,
    user_id: uuid::Uuid,
    guilds: &[Guild],
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM user_guilds WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    if !guilds.is_empty() {
        let ids: Vec<String> = guilds.iter().map(|g| g.id.clone()).collect();
        let names: Vec<String> = guilds.iter().map(|g| g.name.clone()).collect();
        let counts: Vec<Option<i32>> = guilds
            .iter()
            .map(|g| g.approximate_member_count.map(|n| n as i32))
            .collect();
        sqlx::query(
            "INSERT INTO user_guilds (user_id, guild_id, guild_name, member_count)
             SELECT $1, * FROM UNNEST($2::text[], $3::text[], $4::int4[])
             ON CONFLICT (user_id, guild_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(ids)
        .bind(names)
        .bind(counts)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE users SET servers_connected = true WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// What is being shared right now.
pub async fn status(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
) -> Result<Json<ServersStatus>, ApiError> {
    let connected =
        sqlx::query_scalar::<_, bool>("SELECT servers_connected FROM users WHERE id = $1")
            .bind(uid)
            .fetch_one(&state.db)
            .await?;
    let servers = sqlx::query_scalar::<_, String>(
        "SELECT guild_name FROM user_guilds WHERE user_id = $1 ORDER BY guild_name",
    )
    .bind(uid)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(ServersStatus { connected, servers }))
}

/// Stop sharing and forget every stored server.
pub async fn disconnect(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM user_guilds WHERE user_id = $1")
        .bind(uid)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE users SET servers_connected = false WHERE id = $1")
        .bind(uid)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(FromRow)]
struct PersonRow {
    #[sqlx(flatten)]
    profile: ProfileRow,
    shared_servers: Vec<String>,
}

/// Tomo players who share a server with you. Both sides must have opted in (only opted-in people
/// have stored servers at all), and blocked people never appear.
pub async fn people(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
) -> Result<Json<Vec<ServerPerson>>, ApiError> {
    let rows = sqlx::query_as::<_, PersonRow>(concat!(
        "SELECT * FROM (
            SELECT ",
        profile_cols_p!(),
        ",
                ARRAY(SELECT mg.guild_name FROM user_guilds mg
                      JOIN user_guilds tg ON tg.guild_id = mg.guild_id
                      WHERE mg.user_id = $1 AND tg.user_id = p.user_id
                      ORDER BY mg.guild_name) AS shared_servers
            FROM profiles p
            WHERE p.user_id <> $1
              AND NOT EXISTS (SELECT 1 FROM blocks b
                              WHERE (b.blocker_id = $1 AND b.blocked_id = p.user_id)
                                 OR (b.blocker_id = p.user_id AND b.blocked_id = $1))
         ) t
         WHERE cardinality(t.shared_servers) > 0
         ORDER BY cardinality(t.shared_servers) DESC, t.display_name
         LIMIT 50"
    ))
    .bind(uid)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ServerPerson {
                profile: r.profile.into(),
                shared_servers: r.shared_servers,
            })
            .collect(),
    ))
}
