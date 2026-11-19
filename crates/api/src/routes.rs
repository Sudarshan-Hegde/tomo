use axum::{extract::State, http::HeaderMap, Json};
use sqlx::FromRow;
use tomo_shared::{
    AuthResponse, DevLoginRequest, DiscoverCard, Health, Me, MatchedWith, Profile, ProfileInput, SwipeRequest,
    SwipeResponse, User,
};
use uuid::Uuid;

use crate::{
    auth::{bearer_token, hash_token, new_token, AuthUser},
    error::ApiError,
    ranking::{self, Candidate},
    safety::is_blocked,
    AppState,
};

const DECK_SIZE: usize = 20;
/// Candidates pulled (in random order) before ranking picks the best ones.
const CANDIDATE_POOL: i64 = 300;

#[derive(FromRow)]
pub struct ProfileRow {
    pub user_id: Uuid,
    pub display_name: String,
    pub bio: String,
    pub avatar: String,
    pub hue: i16,
    pub rank: String,
    pub region: String,
    pub playstyle: String,
    pub mic: bool,
    pub games: Vec<String>,
}

impl From<ProfileRow> for Profile {
    fn from(r: ProfileRow) -> Self {
        Profile {
            user_id: r.user_id,
            display_name: r.display_name,
            bio: r.bio,
            avatar: r.avatar,
            hue: r.hue.clamp(0, 360) as u16,
            games: r.games,
            rank: r.rank,
            region: r.region,
            playstyle: r.playstyle,
            mic: r.mic,
        }
    }
}

pub async fn load_profile(db: &sqlx::PgPool, user_id: Uuid) -> Result<Option<Profile>, ApiError> {
    let row = sqlx::query_as::<_, ProfileRow>(concat!("SELECT ", profile_cols!(), " FROM profiles WHERE user_id = $1"))
        .bind(user_id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(Profile::from))
}

/// Everything except signing in requires a profile (which also means the 18+ box was ticked).
pub async fn require_profile(db: &sqlx::PgPool, user_id: Uuid) -> Result<Profile, ApiError> {
    load_profile(db, user_id).await?.ok_or(ApiError::Conflict("Create your profile first."))
}

// ───────── Health ─────────

pub async fn health(State(state): State<AppState>) -> Json<Health> {
    let database = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.db).await.is_ok();
    Json(Health { status: "ok".into(), database })
}

// ───────── Auth ─────────

/// DEV ONLY: sign in as any username. Disabled unless DEV_AUTH=true.
/// Replaced by Discord OAuth; must never be enabled in production.
pub async fn dev_login(
    State(state): State<AppState>,
    Json(req): Json<DevLoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    if !state.dev_auth {
        return Err(ApiError::Forbidden("Dev login is disabled."));
    }
    let username = req.username.trim().to_string();
    let len = username.chars().count();
    if !(2..=24).contains(&len)
        || !username.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(ApiError::BadRequest(
            "Username must be 2-24 characters: letters, numbers, _ - .".into(),
        ));
    }

    let (id, username): (Uuid, String) = sqlx::query_as(
        "INSERT INTO users (username) VALUES ($1)
         ON CONFLICT ((lower(username))) DO UPDATE SET username = users.username
         RETURNING id, username",
    )
    .bind(&username)
    .fetch_one(&state.db)
    .await?;

    let token = new_token();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '30 days')")
        .bind(hash_token(&token))
        .bind(id)
        .execute(&state.db)
        .await?;

    Ok(Json(AuthResponse { token, user: User { id, username } }))
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<(), ApiError> {
    if let Some(token) = bearer_token(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(hash_token(token))
            .execute(&state.db)
            .await?;
    }
    Ok(())
}

pub async fn me(State(state): State<AppState>, AuthUser(uid): AuthUser) -> Result<Json<Me>, ApiError> {
    let (id, username, adult_confirmed): (Uuid, String, bool) =
        sqlx::query_as("SELECT id, username, adult_confirmed_at IS NOT NULL FROM users WHERE id = $1")
            .bind(uid)
            .fetch_one(&state.db)
            .await?;
    let profile = load_profile(&state.db, uid).await?;
    Ok(Json(Me { user: User { id, username }, profile, adult_confirmed }))
}

// ───────── Profile ─────────

pub async fn put_profile(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
    Json(input): Json<ProfileInput>,
) -> Result<Json<Profile>, ApiError> {
    input.validate().map_err(ApiError::BadRequest)?;

    // 18+ gate: must be ticked the first time, and is recorded on the account.
    let already_confirmed =
        sqlx::query_scalar::<_, bool>("SELECT adult_confirmed_at IS NOT NULL FROM users WHERE id = $1")
            .bind(uid)
            .fetch_one(&state.db)
            .await?;
    if !already_confirmed {
        if !input.adult_confirmed {
            return Err(ApiError::BadRequest("You must confirm that you are 18 or older.".into()));
        }
        sqlx::query("UPDATE users SET adult_confirmed_at = now() WHERE id = $1")
            .bind(uid)
            .execute(&state.db)
            .await?;
    }

    let row = sqlx::query_as::<_, ProfileRow>(concat!(
        "INSERT INTO profiles (user_id, display_name, bio, avatar, hue, rank, region, playstyle, mic, games)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (user_id) DO UPDATE SET
            display_name = EXCLUDED.display_name, bio = EXCLUDED.bio, avatar = EXCLUDED.avatar,
            hue = EXCLUDED.hue, rank = EXCLUDED.rank, region = EXCLUDED.region,
            playstyle = EXCLUDED.playstyle, mic = EXCLUDED.mic, games = EXCLUDED.games,
            updated_at = now()
         RETURNING ",
        profile_cols!()
    ))
    .bind(uid)
    .bind(input.display_name.trim())
    .bind(input.bio.trim())
    .bind(&input.avatar)
    .bind(input.hue as i16)
    .bind(&input.rank)
    .bind(&input.region)
    .bind(&input.playstyle)
    .bind(input.mic)
    .bind(&input.games)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(row.into()))
}

// ───────── Discover / swipe ─────────

#[derive(FromRow)]
struct CandidateRow {
    #[sqlx(flatten)]
    profile: ProfileRow,
    likes_me: bool,
}

pub async fn discover(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
) -> Result<Json<Vec<DiscoverCard>>, ApiError> {
    let me = require_profile(&state.db, uid).await?;

    // Everyone I haven't swiped on, minus blocks in either direction, in random order.
    let rows = sqlx::query_as::<_, CandidateRow>(concat!(
        "SELECT ",
        profile_cols_p!(),
        ",
            EXISTS (SELECT 1 FROM swipes s WHERE s.swiper_id = p.user_id AND s.target_id = $1 AND s.liked) AS likes_me
         FROM profiles p
         WHERE p.user_id <> $1
           AND NOT EXISTS (SELECT 1 FROM swipes s WHERE s.swiper_id = $1 AND s.target_id = p.user_id)
           AND NOT EXISTS (SELECT 1 FROM blocks b
                           WHERE (b.blocker_id = $1 AND b.blocked_id = p.user_id)
                              OR (b.blocker_id = p.user_id AND b.blocked_id = $1))
         ORDER BY random()
         LIMIT $2"
    ))
    .bind(uid)
    .bind(CANDIDATE_POOL)
    .fetch_all(&state.db)
    .await?;

    let candidates = rows
        .into_iter()
        .map(|r| Candidate { profile: r.profile.into(), likes_me: r.likes_me })
        .collect();
    Ok(Json(ranking::rank_candidates(&me, candidates, DECK_SIZE)))
}

pub async fn swipe(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
    Json(req): Json<SwipeRequest>,
) -> Result<Json<SwipeResponse>, ApiError> {
    if req.target_id == uid {
        return Err(ApiError::BadRequest("You cannot swipe on yourself.".into()));
    }
    require_profile(&state.db, uid).await?;
    let target = load_profile(&state.db, req.target_id)
        .await?
        .ok_or(ApiError::NotFound("That player does not exist."))?;
    if is_blocked(&state.db, uid, req.target_id).await? {
        return Err(ApiError::NotFound("That player does not exist."));
    }

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO swipes (swiper_id, target_id, liked) VALUES ($1, $2, $3)
         ON CONFLICT (swiper_id, target_id) DO UPDATE SET liked = EXCLUDED.liked, created_at = now()",
    )
    .bind(uid)
    .bind(req.target_id)
    .bind(req.liked)
    .execute(&mut *tx)
    .await?;

    let mut matched = None;
    if req.liked {
        let they_like_me = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM swipes WHERE swiper_id = $1 AND target_id = $2 AND liked)",
        )
        .bind(req.target_id)
        .bind(uid)
        .fetch_one(&mut *tx)
        .await?;

        if they_like_me {
            // Store the pair ordered so (a, b) and (b, a) are the same match.
            let (a, b) = if uid < req.target_id { (uid, req.target_id) } else { (req.target_id, uid) };
            // The no-op update makes RETURNING work whether the row is new or already existed.
            let match_id: Uuid = sqlx::query_scalar(
                "INSERT INTO matches (user_a, user_b) VALUES ($1, $2)
                 ON CONFLICT (user_a, user_b) DO UPDATE SET user_a = EXCLUDED.user_a
                 RETURNING id",
            )
            .bind(a)
            .bind(b)
            .fetch_one(&mut *tx)
            .await?;
            matched = Some(MatchedWith { match_id, profile: target });
        }
    }
    tx.commit().await?;
    Ok(Json(SwipeResponse { matched }))
}
