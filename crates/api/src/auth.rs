use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts, HeaderMap},
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{error::ApiError, AppState};

/// The signed-in user, resolved from `Authorization: Bearer <token>`.
pub struct AuthUser(pub Uuid);

pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|t| !t.is_empty())
}

/// Tokens are stored hashed, so a database leak does not leak live sessions.
pub fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 244 bits of OS randomness (two v4 UUIDs), hex encoded.
pub fn new_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// Start a 30-day session for `user_id` and return the raw token (only its hash is stored).
pub async fn create_session(db: &sqlx::PgPool, user_id: Uuid) -> Result<String, sqlx::Error> {
    let token = new_token();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '30 days')")
        .bind(hash_token(&token))
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(token)
}

/// Resolve a raw bearer token to its user, if the session is valid and unexpired.
pub async fn user_for_token(db: &sqlx::PgPool, token: &str) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT user_id FROM sessions WHERE token_hash = $1 AND expires_at > now()",
    )
    .bind(hash_token(token))
    .fetch_optional(db)
    .await
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers).ok_or(ApiError::Unauthorized)?;
        let user_id = user_for_token(&state.db, token)
            .await?
            .ok_or(ApiError::Unauthorized)?;
        Ok(AuthUser(user_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_long() {
        let a = new_token();
        let b = new_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn hash_is_deterministic_and_not_the_token() {
        let t = new_token();
        assert_eq!(hash_token(&t), hash_token(&t));
        assert_ne!(hash_token(&t), t);
    }
}
