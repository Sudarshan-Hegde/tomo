//! Friends via invite links. Everyone has one link. Whoever signs up through it (or opens it while
//! signed in) becomes that person's friend, and friends are boosted in each other's Discover deck.

use axum::{
    extract::{Path, State},
    Json,
};
use sqlx::FromRow;
use tomo_shared::{AcceptInviteRequest, AcceptInviteResponse, Friend, InviteInfo, InvitePreview};
use uuid::Uuid;

use crate::{auth::AuthUser, error::ApiError, routes::ProfileRow, safety::is_blocked, AppState};

const CODE_LEN: usize = 10;

/// Codes are 10 lowercase hex characters. Checking the shape first means odd input never reaches the database.
fn valid_code(code: &str) -> bool {
    code.len() == CODE_LEN && code.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

fn new_code() -> String {
    Uuid::new_v4().simple().to_string()[..CODE_LEN].to_string()
}

/// The pair as stored: smaller id first, so (a, b) and (b, a) are the same friendship.
pub fn ordered(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Make sure this person has an invite code, creating one the first time.
async fn ensure_invite_code(db: &sqlx::PgPool, uid: Uuid) -> Result<String, sqlx::Error> {
    for _ in 0..5 {
        if let Some(code) =
            sqlx::query_scalar::<_, Option<String>>("SELECT invite_code FROM users WHERE id = $1")
                .bind(uid)
                .fetch_one(db)
                .await?
        {
            return Ok(code);
        }
        let result = sqlx::query_scalar::<_, String>(
            "UPDATE users SET invite_code = $2 WHERE id = $1 AND invite_code IS NULL RETURNING invite_code",
        )
        .bind(uid)
        .bind(new_code())
        .fetch_optional(db)
        .await;
        match result {
            Ok(Some(code)) => return Ok(code),
            Ok(None) => continue, // someone (another tab) got there first; read theirs
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => continue, // code collision; try another
            Err(e) => return Err(e),
        }
    }
    Err(sqlx::Error::Protocol(
        "could not create an invite code".into(),
    ))
}

/// Your own shareable invite link.
pub async fn my_invite(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
) -> Result<Json<InviteInfo>, ApiError> {
    let code = ensure_invite_code(&state.db, uid).await?;
    let url = format!("{}/invite/{}", state.config.frontend_url, code);
    Ok(Json(InviteInfo { code, url }))
}

/// Public: who is inviting me? Shows only a name and picture, so the link feels personal
/// without exposing anything else about them.
pub async fn preview(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<InvitePreview>, ApiError> {
    if !valid_code(&code) {
        return Err(ApiError::NotFound("That invite link is not valid."));
    }
    let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT COALESCE(p.display_name, u.discord_global_name, u.username), p.avatar, p.avatar_url
         FROM users u LEFT JOIN profiles p ON p.user_id = u.id
         WHERE u.invite_code = $1",
    )
    .bind(&code)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("That invite link is not valid."))?;
    Ok(Json(InvitePreview {
        inviter_name: row.0,
        inviter_emoji: row.1,
        inviter_avatar_url: row.2,
    }))
}

/// Use someone's invite link. Safe to repeat.
pub async fn accept(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
    Json(req): Json<AcceptInviteRequest>,
) -> Result<Json<AcceptInviteResponse>, ApiError> {
    if !valid_code(&req.code) {
        return Err(ApiError::NotFound("That invite link is not valid."));
    }
    let (inviter, friend_name) = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT u.id, COALESCE(p.display_name, u.discord_global_name, u.username)
         FROM users u LEFT JOIN profiles p ON p.user_id = u.id
         WHERE u.invite_code = $1",
    )
    .bind(&req.code)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("That invite link is not valid."))?;

    if inviter == uid {
        return Err(ApiError::BadRequest(
            "That is your own invite link. Send it to a friend instead.".into(),
        ));
    }
    // A blocked person looks like a link that does not exist, so blocks are not revealed.
    if is_blocked(&state.db, uid, inviter).await? {
        return Err(ApiError::NotFound("That invite link is not valid."));
    }

    let (a, b) = ordered(uid, inviter);
    let created = sqlx::query(
        "INSERT INTO friendships (user_a, user_b) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(a)
    .bind(b)
    .execute(&state.db)
    .await?
    .rows_affected();
    if created > 0 {
        // Remember who brought this person in (first invite wins).
        sqlx::query("UPDATE users SET invited_by = $2 WHERE id = $1 AND invited_by IS NULL")
            .bind(uid)
            .bind(inviter)
            .execute(&state.db)
            .await?;
    }
    Ok(Json(AcceptInviteResponse {
        friend_name,
        already_friends: created == 0,
    }))
}

#[derive(FromRow)]
struct FriendRow {
    #[sqlx(flatten)]
    profile: ProfileRow,
    since: chrono::DateTime<chrono::Utc>,
}

/// Your friends (people who have a profile), newest first. Blocked people are left out.
pub async fn list(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
) -> Result<Json<Vec<Friend>>, ApiError> {
    let rows = sqlx::query_as::<_, FriendRow>(concat!(
        "SELECT ",
        profile_cols_p!(),
        ", f.created_at AS since
         FROM friendships f
         JOIN profiles p ON p.user_id = CASE WHEN f.user_a = $1 THEN f.user_b ELSE f.user_a END
         WHERE (f.user_a = $1 OR f.user_b = $1)
           AND NOT EXISTS (SELECT 1 FROM blocks b
                           WHERE (b.blocker_id = $1 AND b.blocked_id = p.user_id)
                              OR (b.blocker_id = p.user_id AND b.blocked_id = $1))
         ORDER BY f.created_at DESC"
    ))
    .bind(uid)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| Friend {
                profile: r.profile.into(),
                since_ms: r.since.timestamp_millis(),
            })
            .collect(),
    ))
}

/// End a friendship (they are not told).
pub async fn remove(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
    Path(other): Path<Uuid>,
) -> Result<(), ApiError> {
    let (a, b) = ordered(uid, other);
    sqlx::query("DELETE FROM friendships WHERE user_a = $1 AND user_b = $2")
        .bind(a)
        .bind(b)
        .execute(&state.db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_have_a_fixed_safe_shape() {
        assert!(valid_code("0123456789"));
        assert!(valid_code("abcdef0123"));
        for bad in [
            "",
            "abc",
            "0123456789a",
            "ABCDEF0123",
            "012345678g",
            "../../etc/",
            "0123 56789",
            "012345678é",
        ] {
            assert!(!valid_code(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn generated_codes_pass_their_own_check_and_differ() {
        let (a, b) = (new_code(), new_code());
        assert!(valid_code(&a) && valid_code(&b));
        assert_ne!(a, b);
    }

    #[test]
    fn a_friendship_is_the_same_whichever_way_round() {
        let (x, y) = (Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(ordered(x, y), ordered(y, x));
        let (a, b) = ordered(x, y);
        assert!(a < b);
    }
}
