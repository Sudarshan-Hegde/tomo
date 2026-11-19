//! Talks to Discord: builds the approval link, turns a login code into a token, and reads who the
//! person is (and, only if they opted in, which small servers they are in).

use serde::Deserialize;

use crate::config::DiscordConfig;

/// Servers bigger than this are ignored: in a huge server, "we share it" says nothing about people
/// and exposing it would feel invasive.
pub const MAX_SERVER_MEMBERS: u32 = 2000;
/// A cap on how many servers one person can share.
pub const MAX_SERVERS: usize = 100;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct DiscordUser {
    /// Discord's permanent numeric id (as a string). This is what identifies the account.
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
    /// Hash of the custom avatar image, or None if they never set one.
    #[serde(default)]
    pub avatar: Option<String>,
}

impl DiscordUser {
    /// The friendly name people see on Discord (falls back to the username).
    pub fn display_name(&self) -> String {
        self.global_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&self.username)
            .to_string()
    }

    /// The avatar picture address, always on Discord's own image host.
    ///
    /// Built here from the id and hash (never copied from client input), and both are checked
    /// for odd characters first, so a strange value can't produce a different address.
    pub fn avatar_url(&self) -> String {
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 25
            && self.id.chars().all(|c| c.is_ascii_digit());
        let hash_ok = |h: &str| {
            !h.is_empty()
                && h.len() <= 64
                && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        };
        match &self.avatar {
            Some(hash) if id_ok && hash_ok(hash) => {
                format!(
                    "https://cdn.discordapp.com/avatars/{}/{}.png?size=256",
                    self.id, hash
                )
            }
            // No custom avatar: Discord shows one of six default pictures, picked from the id.
            _ => {
                let index = self.id.parse::<u64>().map(|n| (n >> 22) % 6).unwrap_or(0);
                format!("https://cdn.discordapp.com/embed/avatars/{index}.png")
            }
        }
    }
}

/// One of the person's Discord servers.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Guild {
    pub id: String,
    pub name: String,
    /// Only present because we ask for counts (`with_counts=true`).
    #[serde(default)]
    pub approximate_member_count: Option<u32>,
}

/// Keep only small community servers with sane ids and names.
pub fn community_servers(guilds: Vec<Guild>) -> Vec<Guild> {
    guilds
        .into_iter()
        .filter(|g| {
            !g.id.is_empty() && g.id.len() <= 25 && g.id.chars().all(|c| c.is_ascii_digit())
        })
        .filter(|g| matches!(g.approximate_member_count, Some(n) if n <= MAX_SERVER_MEMBERS))
        .map(|g| Guild {
            name: g.name.trim().chars().take(100).collect(),
            ..g
        })
        .filter(|g| !g.name.is_empty())
        .take(MAX_SERVERS)
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum DiscordError {
    /// Discord refused: bad, expired or reused code, or wrong app credentials.
    Rejected,
    /// We could not get a usable answer (network trouble, Discord outage).
    Unavailable,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// The Discord page where the person approves the login. By default only the `identify` scope is
/// requested, so we never see email or servers. `guilds` is added only when the person ticked the
/// "find players from my servers" box before leaving for Discord.
pub fn authorize_url(
    cfg: &DiscordConfig,
    redirect_uri: &str,
    state: &str,
    with_servers: bool,
) -> String {
    let mut url =
        reqwest::Url::parse(&cfg.authorize_url).expect("DISCORD_AUTHORIZE_URL must be a valid URL");
    url.query_pairs_mut()
        .append_pair("client_id", &cfg.client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair(
            "scope",
            if with_servers {
                "identify guilds"
            } else {
                "identify"
            },
        )
        .append_pair("state", state);
    url.to_string()
}

fn unavailable(e: reqwest::Error) -> DiscordError {
    tracing::warn!("discord request failed: {e}");
    DiscordError::Unavailable
}

/// Trade the one-time `code` for an access token.
pub async fn exchange_code(
    http: &reqwest::Client,
    cfg: &DiscordConfig,
    redirect_uri: &str,
    code: &str,
) -> Result<String, DiscordError> {
    let resp = http
        .post(format!("{}/oauth2/token", cfg.api_base))
        .form(&[
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await
        .map_err(unavailable)?;
    let status = resp.status();
    if status.is_client_error() {
        tracing::warn!("discord rejected the code exchange: HTTP {status}");
        return Err(DiscordError::Rejected);
    }
    if !status.is_success() {
        tracing::warn!("discord token endpoint failed: HTTP {status}");
        return Err(DiscordError::Unavailable);
    }
    let token: TokenResponse = resp.json().await.map_err(unavailable)?;
    Ok(token.access_token)
}

/// Who does this token belong to?
pub async fn fetch_user(
    http: &reqwest::Client,
    cfg: &DiscordConfig,
    token: &str,
) -> Result<DiscordUser, DiscordError> {
    let resp = http
        .get(format!("{}/users/@me", cfg.api_base))
        .bearer_auth(token)
        .send()
        .await
        .map_err(unavailable)?;
    if !resp.status().is_success() {
        tracing::warn!("discord profile lookup failed: HTTP {}", resp.status());
        return Err(DiscordError::Unavailable);
    }
    resp.json::<DiscordUser>().await.map_err(unavailable)
}

/// The person's servers, with member counts. Needs the `guilds` scope.
pub async fn fetch_guilds(
    http: &reqwest::Client,
    cfg: &DiscordConfig,
    token: &str,
) -> Result<Vec<Guild>, DiscordError> {
    let resp = http
        .get(format!("{}/users/@me/guilds", cfg.api_base))
        .query(&[("with_counts", "true"), ("limit", "200")])
        .bearer_auth(token)
        .send()
        .await
        .map_err(unavailable)?;
    if !resp.status().is_success() {
        tracing::warn!("discord server list failed: HTTP {}", resp.status());
        return Err(DiscordError::Unavailable);
    }
    resp.json::<Vec<Guild>>().await.map_err(unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::Form,
        http::{HeaderMap, StatusCode},
        routing::{get, post},
        Json, Router,
    };
    use std::collections::HashMap;

    /// A stand-in for Discord that accepts exactly one code and one token.
    async fn spawn_fake_discord() -> String {
        async fn token(
            Form(f): Form<HashMap<String, String>>,
        ) -> Result<Json<serde_json::Value>, StatusCode> {
            let ok = f.get("code").map(String::as_str) == Some("good-code")
                && f.get("client_secret").map(String::as_str) == Some("s3cret")
                && f.get("grant_type").map(String::as_str) == Some("authorization_code");
            if ok {
                Ok(Json(
                    serde_json::json!({ "access_token": "tok-123", "token_type": "Bearer" }),
                ))
            } else {
                Err(StatusCode::BAD_REQUEST)
            }
        }
        fn authed(headers: &HeaderMap) -> bool {
            headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer tok-123")
        }
        async fn me(headers: HeaderMap) -> Result<Json<serde_json::Value>, StatusCode> {
            if authed(&headers) {
                Ok(Json(
                    serde_json::json!({ "id": "42", "username": "kitsune", "global_name": "Kitsune!", "avatar": "abc123def" }),
                ))
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        async fn guilds(headers: HeaderMap) -> Result<Json<serde_json::Value>, StatusCode> {
            if authed(&headers) {
                Ok(Json(serde_json::json!([
                    { "id": "1", "name": "Tiny Clan", "approximate_member_count": 12, "owner": false },
                    { "id": "2", "name": "Huge Public Server", "approximate_member_count": 900000 }
                ])))
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        let app = Router::new()
            .route("/oauth2/token", post(token))
            .route("/users/@me", get(me))
            .route("/users/@me/guilds", get(guilds));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn cfg(base: &str) -> DiscordConfig {
        DiscordConfig {
            client_id: "app-1".into(),
            client_secret: "s3cret".into(),
            authorize_url: "https://discord.com/oauth2/authorize".into(),
            api_base: base.into(),
        }
    }

    #[test]
    fn authorize_url_asks_for_identify_only_by_default() {
        let url = authorize_url(
            &cfg("x"),
            "http://localhost:8080/auth/discord/callback",
            "abc123",
            false,
        );
        let parsed = reqwest::Url::parse(&url).unwrap();
        let q: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(parsed.host_str(), Some("discord.com"));
        assert_eq!(q["client_id"], "app-1");
        assert_eq!(
            q["redirect_uri"],
            "http://localhost:8080/auth/discord/callback"
        );
        assert_eq!(q["response_type"], "code");
        assert_eq!(
            q["scope"], "identify",
            "must not ask for email or servers unless the person opted in"
        );
        assert_eq!(q["state"], "abc123");
        assert!(
            !url.contains("s3cret"),
            "the secret must never be in a link"
        );
    }

    #[test]
    fn the_servers_permission_is_requested_only_when_opted_in() {
        let url = authorize_url(&cfg("x"), "http://x/cb", "st", true);
        let q: HashMap<_, _> = reqwest::Url::parse(&url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
        assert_eq!(q["scope"], "identify guilds");
        assert!(!q["scope"].contains("email"));
    }

    async fn identity(base: &str, code: &str) -> Result<DiscordUser, DiscordError> {
        let http = reqwest::Client::new();
        let token = exchange_code(&http, &cfg(base), "http://x/cb", code).await?;
        fetch_user(&http, &cfg(base), &token).await
    }

    #[tokio::test]
    async fn a_good_code_returns_the_discord_identity() {
        let base = spawn_fake_discord().await;
        assert_eq!(
            identity(&base, "good-code").await.unwrap(),
            DiscordUser {
                id: "42".into(),
                username: "kitsune".into(),
                global_name: Some("Kitsune!".into()),
                avatar: Some("abc123def".into())
            }
        );
    }

    #[tokio::test]
    async fn a_bad_code_is_rejected() {
        let base = spawn_fake_discord().await;
        assert_eq!(
            identity(&base, "stolen-or-expired").await,
            Err(DiscordError::Rejected)
        );
    }

    #[tokio::test]
    async fn wrong_app_secret_is_rejected() {
        let base = spawn_fake_discord().await;
        let mut bad = cfg(&base);
        bad.client_secret = "wrong".into();
        let err = exchange_code(&reqwest::Client::new(), &bad, "http://x/cb", "good-code").await;
        assert_eq!(err, Err(DiscordError::Rejected));
    }

    #[tokio::test]
    async fn an_unreachable_discord_is_unavailable_not_rejected() {
        // Nothing listens on this port, so this must read as "try again", never as "bad login".
        assert_eq!(
            identity("http://127.0.0.1:1", "good-code").await,
            Err(DiscordError::Unavailable)
        );
    }

    #[tokio::test]
    async fn the_server_list_comes_back_with_counts() {
        let base = spawn_fake_discord().await;
        let http = reqwest::Client::new();
        let token = exchange_code(&http, &cfg(&base), "http://x/cb", "good-code")
            .await
            .unwrap();
        let guilds = fetch_guilds(&http, &cfg(&base), &token).await.unwrap();
        assert_eq!(guilds.len(), 2);
        assert_eq!(guilds[0].approximate_member_count, Some(12));
        assert_eq!(
            fetch_guilds(&http, &cfg(&base), "wrong-token").await,
            Err(DiscordError::Unavailable)
        );
    }

    fn guild(id: &str, name: &str, members: Option<u32>) -> Guild {
        Guild {
            id: id.into(),
            name: name.into(),
            approximate_member_count: members,
        }
    }

    #[test]
    fn only_small_communities_are_kept() {
        let kept = community_servers(vec![
            guild("1", "Tiny Clan", Some(12)),
            guild("2", "Huge Public Server", Some(900_000)),
            guild("3", "Edge", Some(MAX_SERVER_MEMBERS)),
            guild("4", "One Over", Some(MAX_SERVER_MEMBERS + 1)),
            guild("5", "No Count", None),
        ]);
        let names: Vec<_> = kept.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["Tiny Clan", "Edge"]);
    }

    #[test]
    fn odd_server_data_is_cleaned_or_dropped() {
        let kept = community_servers(vec![
            guild("../1", "bad id", Some(5)),
            guild("", "empty id", Some(5)),
            guild("6", "   ", Some(5)),
            guild("7", "  Padded  ", Some(5)),
            guild("8", &"x".repeat(500), Some(5)),
        ]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].name, "Padded");
        assert_eq!(kept[1].name.chars().count(), 100);
    }

    #[test]
    fn nobody_can_share_more_than_the_cap() {
        let many: Vec<Guild> = (1..=300)
            .map(|i| guild(&i.to_string(), "s", Some(5)))
            .collect();
        assert_eq!(community_servers(many).len(), MAX_SERVERS);
    }

    fn user(id: &str, avatar: Option<&str>, global: Option<&str>) -> DiscordUser {
        DiscordUser {
            id: id.into(),
            username: "kitsune".into(),
            global_name: global.map(Into::into),
            avatar: avatar.map(Into::into),
        }
    }

    #[test]
    fn a_custom_avatar_points_at_discords_image_host() {
        let u = user(
            "80351110224678912",
            Some("8342729096ea3675442027381ff50dfe"),
            None,
        );
        assert_eq!(
            u.avatar_url(),
            "https://cdn.discordapp.com/avatars/80351110224678912/8342729096ea3675442027381ff50dfe.png?size=256"
        );
    }

    #[test]
    fn animated_avatars_use_the_still_png() {
        let u = user("123", Some("a_8342729096ea"), None);
        assert!(
            u.avatar_url().contains("/avatars/123/a_8342729096ea.png"),
            "{}",
            u.avatar_url()
        );
    }

    #[test]
    fn no_custom_avatar_gives_one_of_the_six_default_pictures() {
        // (id >> 22) % 6: 80351110224678912 >> 22 = 19157197529, and 19157197529 % 6 = 5
        let u = user("80351110224678912", None, None);
        assert_eq!(
            u.avatar_url(),
            "https://cdn.discordapp.com/embed/avatars/5.png"
        );
        let idx = |id: &str| user(id, None, None).avatar_url();
        assert!(idx("0").ends_with("/embed/avatars/0.png"));
    }

    #[test]
    fn strange_ids_or_hashes_can_never_change_the_host_or_path() {
        for bad in ["../../evil", "abc/../x", "a b", "x?y=1", "x#y", "", "é"] {
            let u = user("123", Some(bad), None);
            let url = u.avatar_url();
            assert!(
                url.starts_with("https://cdn.discordapp.com/embed/avatars/"),
                "{bad:?} gave {url}"
            );
            let u2 = user(bad, Some("abc"), None);
            assert!(
                u2.avatar_url()
                    .starts_with("https://cdn.discordapp.com/embed/avatars/"),
                "id {bad:?}"
            );
        }
        let long = "a".repeat(65);
        assert!(user("123", Some(&long), None)
            .avatar_url()
            .contains("/embed/avatars/"));
    }

    #[test]
    fn display_name_prefers_the_friendly_name() {
        assert_eq!(user("1", None, Some("Kitsune!")).display_name(), "Kitsune!");
        assert_eq!(user("1", None, Some("   ")).display_name(), "kitsune");
        assert_eq!(user("1", None, None).display_name(), "kitsune");
    }
}
