//! All deployment-specific settings, read once from environment variables.
//! Defaults are safe for local development and restrictive for everything else.

use axum::http::HeaderValue;

#[derive(Clone, Debug)]
pub struct DiscordConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Where we send people to approve the login.
    pub authorize_url: String,
    /// Where we exchange the code and read the profile. Overridable so tests can use a fake.
    pub api_base: String,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub port: String,
    /// DEV ONLY username login. Off unless DEV_AUTH=true.
    pub dev_auth: bool,
    /// Browser origins allowed to call the API (CORS), e.g. https://tomo.pages.dev
    pub allowed_origins: Vec<HeaderValue>,
    /// Where the browser app lives. Discord login sends people back here.
    pub frontend_url: String,
    /// Where this API is reachable from the internet. Used to build the Discord redirect address.
    pub public_api_url: String,
    /// None means Discord login is not configured, and the button stays disabled.
    pub discord: Option<DiscordConfig>,
    /// Steam's public store address (overridable so tests can use a fake).
    pub steam_store_base: String,
    /// Steam's public API address (overridable so tests can use a fake).
    pub steam_api_base: String,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn trim_slash(s: String) -> String {
    s.trim_end_matches('/').to_string()
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url =
            env("DATABASE_URL").ok_or_else(|| anyhow::anyhow!("DATABASE_URL is not set"))?;
        let port = env("PORT").unwrap_or_else(|| "8080".into());

        let frontend_url =
            trim_slash(env("FRONTEND_URL").unwrap_or_else(|| "http://localhost:3001".into()));
        let public_api_url =
            trim_slash(env("PUBLIC_API_URL").unwrap_or_else(|| format!("http://localhost:{port}")));

        // Comma-separated. The frontend is always allowed; extras (like a local dev address) are optional.
        let raw_origins = env("ALLOWED_ORIGINS").unwrap_or_else(|| {
            format!("{frontend_url},http://127.0.0.1:3001,http://localhost:3001")
        });
        let allowed_origins = raw_origins
            .split(',')
            .map(|o| o.trim().trim_end_matches('/'))
            .filter(|o| !o.is_empty())
            .map(|o| HeaderValue::from_str(o).map_err(|e| anyhow::anyhow!("bad origin {o:?}: {e}")))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let discord = match (env("DISCORD_CLIENT_ID"), env("DISCORD_CLIENT_SECRET")) {
            (Some(client_id), Some(client_secret)) => Some(DiscordConfig {
                client_id,
                client_secret,
                authorize_url: env("DISCORD_AUTHORIZE_URL")
                    .unwrap_or_else(|| "https://discord.com/oauth2/authorize".into()),
                api_base: trim_slash(
                    env("DISCORD_API_BASE").unwrap_or_else(|| "https://discord.com/api/v10".into()),
                ),
            }),
            _ => None,
        };

        Ok(Config {
            database_url,
            port,
            dev_auth: env("DEV_AUTH").is_some_and(|v| v == "true"),
            allowed_origins,
            frontend_url,
            public_api_url,
            discord,
            steam_store_base: trim_slash(
                env("STEAM_STORE_BASE").unwrap_or_else(|| "https://store.steampowered.com".into()),
            ),
            steam_api_base: trim_slash(
                env("STEAM_API_BASE").unwrap_or_else(|| "https://api.steampowered.com".into()),
            ),
        })
    }

    /// The address Discord redirects back to. Must match what is registered in the Discord app.
    pub fn discord_redirect_uri(&self) -> String {
        format!("{}/auth/discord/callback", self.public_api_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_uri_is_built_from_the_public_api_url() {
        let cfg = Config {
            database_url: "x".into(),
            port: "8080".into(),
            dev_auth: false,
            allowed_origins: vec![],
            frontend_url: "https://tomo.pages.dev".into(),
            public_api_url: "https://tomo-api.onrender.com".into(),
            discord: None,
            steam_store_base: String::new(),
            steam_api_base: String::new(),
        };
        assert_eq!(
            cfg.discord_redirect_uri(),
            "https://tomo-api.onrender.com/auth/discord/callback"
        );
    }

    #[test]
    fn trailing_slashes_are_removed() {
        assert_eq!(trim_slash("https://a.example/".into()), "https://a.example");
        assert_eq!(trim_slash("https://a.example".into()), "https://a.example");
    }
}
