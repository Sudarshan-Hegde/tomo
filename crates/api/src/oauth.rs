//! "Continue with Discord": start, callback, and the one-time-code exchange.
//!
//! Flow: browser -> GET /auth/discord -> Discord -> GET /auth/discord/callback
//!       -> redirect to the site with a one-time code -> POST /auth/exchange -> session.

use axum::{
    extract::{Query, State},
    response::Redirect,
    Json,
};
use serde::Deserialize;
use tomo_shared::{AuthConfig, AuthResponse, ExchangeRequest, User};
use uuid::Uuid;

use crate::{
    auth::{create_session, hash_token, new_token},
    discord::{self, DiscordError, DiscordUser},
    error::ApiError,
    AppState,
};

pub async fn auth_config(State(state): State<AppState>) -> Json<AuthConfig> {
    Json(AuthConfig {
        discord: state.config.discord.is_some(),
        dev: state.config.dev_auth,
    })
}

#[derive(Deserialize)]
pub struct StartParams {
    /// `?servers=1` means the person ticked "find players from my Discord servers".
    servers: Option<String>,
}

/// Step 1: remember a random `state`, then send the browser to Discord.
pub async fn discord_start(
    State(state): State<AppState>,
    Query(params): Query<StartParams>,
) -> Result<Redirect, ApiError> {
    let Some(cfg) = &state.config.discord else {
        return Err(ApiError::NotFound(
            "Discord sign-in is not set up on this server.",
        ));
    };
    // Opt-in only: the server list is requested from Discord solely when this is explicitly "1".
    let with_servers = params.servers.as_deref() == Some("1");

    let st = new_token();
    // Tidy up abandoned attempts while we are here.
    sqlx::query("DELETE FROM oauth_states WHERE expires_at < now()")
        .execute(&state.db)
        .await?;
    sqlx::query(
        "INSERT INTO oauth_states (state_hash, expires_at, with_servers) VALUES ($1, now() + interval '10 minutes', $2)",
    )
    .bind(hash_token(&st))
    .bind(with_servers)
    .execute(&state.db)
    .await?;

    Ok(Redirect::to(&discord::authorize_url(
        cfg,
        &state.config.discord_redirect_uri(),
        &st,
        with_servers,
    )))
}

#[derive(Deserialize)]
pub struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    /// Discord sends this instead of `code` when the person clicks "Cancel".
    error: Option<String>,
}

/// Back to the site's login page with a readable message. Never shows raw errors.
fn back_with_error(frontend: &str, message: &str) -> Redirect {
    let mut url = reqwest::Url::parse(&format!("{frontend}/login"))
        .expect("FRONTEND_URL must be a valid URL");
    url.query_pairs_mut().append_pair("error", message);
    Redirect::to(url.as_str())
}

/// Step 2: Discord sends the person back here. Always answers with a redirect to the site.
pub async fn discord_callback(
    State(state): State<AppState>,
    Query(p): Query<CallbackParams>,
) -> Redirect {
    let frontend = state.config.frontend_url.clone();
    let fail = |msg: &str| back_with_error(&frontend, msg);

    let Some(cfg) = &state.config.discord else {
        return fail("Discord sign-in is not set up on this server.");
    };
    if p.error.is_some() {
        return fail("Discord sign-in was cancelled.");
    }
    let (Some(code), Some(st)) = (p.code, p.state) else {
        return fail("The sign-in response was incomplete. Please try again.");
    };

    // The state must be one we issued, unexpired, and it can only be used once.
    // It also remembers whether this sign-in was started with the servers box ticked.
    let consumed = sqlx::query_scalar::<_, bool>(
        "DELETE FROM oauth_states WHERE state_hash = $1 AND expires_at > now() RETURNING with_servers",
    )
    .bind(hash_token(&st))
    .fetch_optional(&state.db)
    .await;
    let with_servers = match consumed {
        Ok(Some(flag)) => flag,
        Ok(None) => return fail("That sign-in expired or did not start here. Please try again."),
        Err(e) => {
            tracing::error!("oauth state lookup: {e}");
            return fail("Something went wrong. Please try again.");
        }
    };

    let redirect_uri = state.config.discord_redirect_uri();
    let token = match discord::exchange_code(&state.http, cfg, &redirect_uri, &code).await {
        Ok(t) => t,
        Err(DiscordError::Rejected) => {
            return fail("Discord did not accept that sign-in. Please try again.")
        }
        Err(DiscordError::Unavailable) => {
            return fail("Could not reach Discord. Please try again in a moment.")
        }
    };
    let discord_user = match discord::fetch_user(&state.http, cfg, &token).await {
        Ok(u) => u,
        Err(_) => return fail("Could not reach Discord. Please try again in a moment."),
    };

    let user_id = match find_or_create_user(&state.db, &discord_user).await {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("find or create discord user: {e}");
            return fail("Something went wrong. Please try again.");
        }
    };

    // Keep their Discord picture and name current: people change them, and every sign-in refreshes.
    if let Err(e) = refresh_discord_details(&state.db, user_id, &discord_user).await {
        tracing::error!("refresh discord details: {e}");
        return fail("Something went wrong. Please try again.");
    }

    // Only if they opted in: remember their small-community servers. Failing to read them must not
    // block the sign-in itself, so it is logged and skipped.
    let mut servers_saved = false;
    if with_servers {
        match discord::fetch_guilds(&state.http, cfg, &token).await {
            Ok(guilds) => match crate::servers::store_guilds(
                &state.db,
                user_id,
                &discord::community_servers(guilds),
            )
            .await
            {
                Ok(()) => servers_saved = true,
                Err(e) => tracing::error!("store guilds: {e}"),
            },
            Err(_) => tracing::warn!("could not read the server list for an opted-in sign-in"),
        }
    }

    // Hand the browser a one-time code instead of the session itself (keeps tokens out of URLs).
    let one_time = new_token();
    let stored = sqlx::query("INSERT INTO login_codes (code_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '2 minutes')")
    .bind(hash_token(&one_time))
    .bind(user_id)
    .execute(&state.db)
    .await;
    if let Err(e) = stored {
        tracing::error!("store login code: {e}");
        return fail("Something went wrong. Please try again.");
    }

    // `next=friends` lets the site land people on the page where their servers show up.
    let next = if servers_saved { "&next=friends" } else { "" };
    Redirect::to(&format!("{frontend}/auth/callback?code={one_time}{next}"))
}

/// Step 3: the site trades the one-time code for a real session.
pub async fn exchange(
    State(state): State<AppState>,
    Json(req): Json<ExchangeRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    // DELETE ... RETURNING makes the code single-use even if two requests race.
    let user_id = sqlx::query_scalar::<_, Uuid>(
        "DELETE FROM login_codes WHERE code_hash = $1 AND expires_at > now() RETURNING user_id",
    )
    .bind(hash_token(&req.code))
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::BadRequest("This sign-in link expired. Please try again.".into()))?;

    let username = sqlx::query_scalar::<_, String>("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    let token = create_session(&state.db, user_id).await?;
    Ok(Json(AuthResponse {
        token,
        user: User {
            id: user_id,
            username,
        },
    }))
}

/// Store the latest Discord avatar and name, and update the picture on the profile if the person
/// chose to show their Discord avatar.
async fn refresh_discord_details(
    db: &sqlx::PgPool,
    user_id: Uuid,
    du: &DiscordUser,
) -> Result<(), sqlx::Error> {
    let avatar_url = du.avatar_url();
    sqlx::query("UPDATE users SET discord_avatar_url = $2, discord_global_name = $3 WHERE id = $1")
        .bind(user_id)
        .bind(&avatar_url)
        .bind(du.display_name())
        .execute(db)
        .await?;
    sqlx::query("UPDATE profiles SET avatar_url = $2 WHERE user_id = $1 AND use_discord_avatar")
        .bind(user_id)
        .bind(&avatar_url)
        .execute(db)
        .await?;
    Ok(())
}

/// Keep letters, digits and `_ - .`; cap the length. Falls back to a neutral name.
fn sanitize_username(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .take(24)
        .collect();
    if cleaned.chars().count() < 2 {
        "warrior".to_string()
    } else {
        cleaned
    }
}

/// The Discord id is the identity. A returning person keeps their account (and any username change
/// on Discord does not rename them). New people get their Discord username, or a variant of it if
/// somebody already took that name here.
async fn find_or_create_user(db: &sqlx::PgPool, du: &DiscordUser) -> Result<Uuid, sqlx::Error> {
    let base = sanitize_username(&du.username);
    for attempt in 0..6 {
        if let Some(id) =
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE discord_id = $1")
                .bind(&du.id)
                .fetch_optional(db)
                .await?
        {
            return Ok(id);
        }
        let name = if attempt == 0 {
            base.clone()
        } else {
            format!("{base}_{}", &Uuid::new_v4().simple().to_string()[..4])
        };
        match sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO users (discord_id, username) VALUES ($1, $2) RETURNING id",
        )
        .bind(&du.id)
        .bind(&name)
        .fetch_one(db)
        .await
        {
            Ok(id) => return Ok(id),
            // Name taken (or a parallel first login of the same person): loop and look again.
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => continue,
            Err(e) => return Err(e),
        }
    }
    Err(sqlx::Error::Protocol(
        "could not allocate a unique username".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_cleaned_and_capped() {
        assert_eq!(sanitize_username("kitsune"), "kitsune");
        assert_eq!(sanitize_username("ki tsu<ne>!"), "kitsune");
        assert_eq!(sanitize_username(&"a".repeat(50)).len(), 24);
        assert_eq!(sanitize_username("!!!"), "warrior");
        assert_eq!(sanitize_username("x"), "warrior");
    }

    #[test]
    fn the_error_redirect_encodes_the_message() {
        let r = back_with_error(
            "https://tomo.pages.dev",
            "Discord sign-in was cancelled & more",
        );
        let resp = axum::response::IntoResponse::into_response(r);
        let loc = resp.headers().get("location").unwrap().to_str().unwrap();
        assert!(loc.starts_with("https://tomo.pages.dev/login?error="));
        assert!(!loc.contains(' '), "spaces must be encoded: {loc}");
        assert!(!loc.contains("& more"), "ampersands must be encoded: {loc}");
    }
}
