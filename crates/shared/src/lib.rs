use serde::{Deserialize, Serialize};
pub use uuid::Uuid;

// ───────── Allowed values: single source of truth for UI and API validation ─────────

/// How many games one profile can list.
pub const MAX_GAMES: usize = 12;
pub const RANKS: [&str; 6] = [
    "Unranked", "Bronze", "Silver", "Gold", "Platinum", "Diamond+",
];
pub const PLAYSTYLES: [&str; 5] = [
    "Chill",
    "Competitive",
    "Support main",
    "Entry fragger",
    "Strategist",
];
pub const REGIONS: [&str; 5] = ["India (IST)", "Europe (CET)", "NA East", "NA West", "SEA"];
pub const AVATARS: [&str; 12] = [
    "🦊", "👹", "🥷", "👻", "👺", "🌑", "⚡", "🌙", "🐺", "🦇", "🗡️", "🏯",
];

/// (key stored in the database, label shown to the user)
pub const REPORT_REASONS: [(&str, &str); 6] = [
    ("harassment", "Harassment or hate"),
    ("spam", "Spam or scam"),
    ("underage", "Appears to be under 18"),
    ("cheating", "Cheating or boosting"),
    ("inappropriate", "Inappropriate content"),
    ("other", "Something else"),
];
pub const MAX_MESSAGE_LEN: usize = 2000;

// ───────── Games (from Steam) ─────────

/// A game from Steam's catalogue. The id is Steam's own app id, which is how games are matched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Game {
    pub app_id: u32,
    pub name: String,
    /// Where Steam keeps this game's banner, after the app id (see `is_valid_banner_path`).
    /// None means "use the classic address", which works for older games.
    #[serde(default)]
    pub banner: Option<String>,
}

/// Games that are not on Steam get ids from here up. Steam's own app ids stay far below this.
pub const OTHER_GAME_BASE: u32 = 9_000_000;

impl Game {
    /// True for the hand-picked games that are not on Steam (no banner image, styled tile instead).
    pub fn is_other(&self) -> bool {
        self.app_id >= OTHER_GAME_BASE
    }

    /// The game's banner on Steam's image host. Built from the numeric id and a path that the
    /// server has already checked, so the host can never be anything else.
    pub fn banner_url(&self) -> String {
        match &self.banner {
            Some(path) if is_valid_banner_path(path) => format!(
                "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/{}/{}",
                self.app_id, path
            ),
            _ => game_banner_url(self.app_id),
        }
    }
}

/// Classic banner address. Works for older games; newer ones need the path Steam reports.
pub fn game_banner_url(app_id: u32) -> String {
    format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{app_id}/header.jpg")
}

/// Accepts exactly `header.jpg` or `<40 lowercase hex characters>/header.jpg`, nothing else.
pub fn is_valid_banner_path(path: &str) -> bool {
    let file = match path.split_once('/') {
        Some((hash, rest)) => {
            if hash.len() != 40
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return false;
            }
            rest
        }
        None => path,
    };
    file == "header.jpg"
}

// ───────── Wire types ─────────

/// Response of GET /health.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Health {
    pub status: String,
    pub database: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    pub username: String,
}

/// A gamer profile as shown on a swipe card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub user_id: Uuid,
    pub display_name: String,
    pub bio: String,
    /// Emoji avatar for now; becomes an image URL once uploads exist.
    pub avatar: String,
    /// A picture (the person's Discord avatar) that is shown instead of the emoji when present.
    #[serde(default)]
    pub avatar_url: Option<String>,
    /// 0-360, tints the avatar gradient.
    pub hue: u16,
    pub games: Vec<Game>,
    pub rank: String,
    pub region: String,
    pub playstyle: String,
    pub mic: bool,
}

/// What a user submits when creating or editing their own profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileInput {
    pub display_name: String,
    pub bio: String,
    pub avatar: String,
    pub hue: u16,
    /// Steam app ids. The server looks up the names, so a client can't invent a game.
    pub games: Vec<u32>,
    pub rank: String,
    pub region: String,
    pub playstyle: String,
    pub mic: bool,
    /// "I am 18 or older". Required the first time a profile is created.
    pub adult_confirmed: bool,
    /// Show my Discord avatar instead of the emoji (only possible when signed in with Discord).
    #[serde(default)]
    pub use_discord_avatar: bool,
}

impl ProfileInput {
    /// Shared by the client (instant feedback) and the API (the real check).
    pub fn validate(&self) -> Result<(), String> {
        let name = self.display_name.trim().chars().count();
        if !(1..=32).contains(&name) {
            return Err("Display name must be 1-32 characters.".into());
        }
        if self.bio.chars().count() > 280 {
            return Err("Bio must be 280 characters or fewer.".into());
        }
        if !AVATARS.contains(&self.avatar.as_str()) {
            return Err("Unknown avatar.".into());
        }
        if self.hue > 360 {
            return Err("Hue must be 0-360.".into());
        }
        if !RANKS.contains(&self.rank.as_str()) {
            return Err("Unknown rank.".into());
        }
        if !REGIONS.contains(&self.region.as_str()) {
            return Err("Unknown region.".into());
        }
        if !PLAYSTYLES.contains(&self.playstyle.as_str()) {
            return Err("Unknown playstyle.".into());
        }
        if self.games.is_empty() {
            return Err("Pick at least one game.".into());
        }
        if self.games.len() > MAX_GAMES {
            return Err(format!("Pick at most {MAX_GAMES} games."));
        }
        let mut seen = std::collections::HashSet::new();
        if !self.games.iter().all(|g| seen.insert(*g)) {
            return Err("A game is listed twice.".into());
        }
        // Steam app ids are small positive numbers; this also keeps them within the database's integer range.
        if self.games.iter().any(|g| *g == 0 || *g > i32::MAX as u32) {
            return Err("Unknown game.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevLoginRequest {
    pub username: String,
}

/// Which sign-in methods this server offers (GET /auth/config).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthConfig {
    pub discord: bool,
    /// Development-only username login.
    pub dev: bool,
}

/// The browser trades the one-time code from the Discord redirect for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeRequest {
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Me {
    pub user: User,
    pub profile: Option<Profile>,
    pub adult_confirmed: bool,
    /// Present when the account was created through Discord.
    pub discord: Option<DiscordInfo>,
}

/// What we know from Discord about a signed-in person.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordInfo {
    pub display_name: String,
    pub avatar_url: String,
}

/// A candidate on the swipe deck, with the reasons it was suggested.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoverCard {
    pub profile: Profile,
    /// Names of the games you both play.
    pub shared_games: Vec<String>,
    /// They joined through your invite link (or you through theirs).
    #[serde(default)]
    pub is_friend: bool,
    /// Discord servers you are both in. Only filled in when both of you opted in to sharing.
    #[serde(default)]
    pub shared_servers: Vec<String>,
    pub same_region: bool,
    /// Steps between the two ranks; None if either player is Unranked.
    pub rank_gap: Option<u8>,
    pub score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwipeRequest {
    pub target_id: Uuid,
    pub liked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchedWith {
    pub match_id: Uuid,
    pub profile: Profile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwipeResponse {
    /// Set when this like completed a mutual match.
    pub matched: Option<MatchedWith>,
}

// ───────── Friends (invite links) ─────────

/// The signed-in person's own invite link.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InviteInfo {
    pub code: String,
    /// The full shareable address, e.g. https://tomo.pages.dev/invite/ab12cd34ef
    pub url: String,
}

/// What a visitor sees before signing in: who invited them. Deliberately minimal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvitePreview {
    pub inviter_name: String,
    pub inviter_emoji: Option<String>,
    pub inviter_avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptInviteRequest {
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptInviteResponse {
    pub friend_name: String,
    /// True if you were already friends (opening the link twice is harmless).
    pub already_friends: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Friend {
    pub profile: Profile,
    /// Unix milliseconds.
    pub since_ms: i64,
}

// ───────── Discord servers (opt-in) ─────────

/// Which of your Discord servers Tomo knows about, if you opted in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServersStatus {
    pub connected: bool,
    /// Names of the servers being shared (small communities only).
    pub servers: Vec<String>,
}

/// Another Tomo player who is in one or more of the same (opted-in) servers as you.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerPerson {
    pub profile: Profile,
    pub shared_servers: Vec<String>,
}

// ───────── Chat ─────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessage {
    pub id: i64,
    pub match_id: Uuid,
    pub sender_id: Uuid,
    pub body: String,
    /// Unix milliseconds.
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchSummary {
    pub match_id: Uuid,
    pub profile: Profile,
    pub last_message: Option<ChatMessage>,
    /// Messages from them that you have not read yet.
    pub unread: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatHistory {
    /// Oldest first.
    pub messages: Vec<ChatMessage>,
    /// The id of the last message the other person has read ("seen").
    pub other_last_read: i64,
    pub has_more: bool,
}

/// Frames sent by the browser over the WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Must be the first frame.
    Auth {
        token: String,
    },
    Send {
        match_id: Uuid,
        body: String,
    },
    Typing {
        match_id: Uuid,
        typing: bool,
    },
    /// I have read everything up to and including this message id.
    Read {
        match_id: Uuid,
        up_to: i64,
    },
}

/// Frames sent by the server over the WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Ready {
        user_id: Uuid,
    },
    Message {
        message: ChatMessage,
    },
    Typing {
        match_id: Uuid,
        user_id: Uuid,
        typing: bool,
    },
    Read {
        match_id: Uuid,
        user_id: Uuid,
        up_to: i64,
    },
    Error {
        message: String,
    },
    Unauthorized,
}

// ───────── Safety ─────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockRequest {
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportRequest {
    pub user_id: Uuid,
    pub reason: String,
    pub details: String,
    pub also_block: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> ProfileInput {
        ProfileInput {
            display_name: "Kitsune".into(),
            bio: "hi".into(),
            avatar: "🦊".into(),
            hue: 10,
            games: vec![730],
            rank: "Gold".into(),
            region: "SEA".into(),
            playstyle: "Chill".into(),
            mic: true,
            adult_confirmed: true,
            use_discord_avatar: false,
        }
    }

    #[test]
    fn accepts_valid_profile() {
        assert!(valid().validate().is_ok());
    }

    #[test]
    fn rejects_bad_values() {
        let mut p = valid();
        p.display_name = "   ".into();
        assert!(p.validate().is_err());

        let mut p = valid();
        p.games = vec![];
        assert!(p.validate().is_err());

        let mut p = valid();
        p.rank = "Mythic".into();
        assert!(p.validate().is_err());

        let mut p = valid();
        p.bio = "x".repeat(281);
        assert!(p.validate().is_err());
    }

    #[test]
    fn game_lists_are_checked() {
        let mut p = valid();
        p.games = vec![730, 570, 1172470];
        assert!(p.validate().is_ok());

        p.games = vec![730, 730];
        assert!(p.validate().is_err(), "duplicates");

        p.games = vec![0];
        assert!(p.validate().is_err(), "app id 0 does not exist");

        p.games = vec![u32::MAX];
        assert!(p.validate().is_err(), "too big for the database");

        p.games = (1..=MAX_GAMES as u32).collect();
        assert!(p.validate().is_ok(), "exactly the maximum is fine");
        p.games.push(9999);
        assert!(p.validate().is_err(), "one over the maximum");
    }

    #[test]
    fn games_outside_steam_are_recognised_by_their_id() {
        let steam = Game {
            app_id: 730,
            name: "Counter-Strike 2".into(),
            banner: None,
        };
        let other = Game {
            app_id: OTHER_GAME_BASE + 1,
            name: "Valorant".into(),
            banner: None,
        };
        assert!(!steam.is_other());
        assert!(other.is_other());
    }

    #[test]
    fn classic_banner_address_when_steam_gave_no_path() {
        let g = Game {
            app_id: 730,
            name: "Counter-Strike 2".into(),
            banner: None,
        };
        assert_eq!(
            g.banner_url(),
            "https://cdn.cloudflare.steamstatic.com/steam/apps/730/header.jpg"
        );
    }

    #[test]
    fn hashed_banner_address_when_steam_gave_one() {
        let hash = "db8e0cf43c9475b96fb9446b4908de79fcb9f619";
        let g = Game {
            app_id: 4262460,
            name: "ROB RUN".into(),
            banner: Some(format!("{hash}/header.jpg")),
        };
        assert_eq!(
            g.banner_url(),
            format!("https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/4262460/{hash}/header.jpg")
        );
    }

    #[test]
    fn only_the_exact_banner_shape_is_accepted() {
        let hash = "db8e0cf43c9475b96fb9446b4908de79fcb9f619";
        assert!(is_valid_banner_path("header.jpg"));
        assert!(is_valid_banner_path(&format!("{hash}/header.jpg")));
        for bad in [
            "",
            "../header.jpg",
            "header.png",
            "x/header.jpg",
            &format!("{hash}/other.jpg"),
            &format!("{hash}/header.jpg?x=1"),
            &format!("{}/header.jpg", hash.to_uppercase()),
            &format!("{hash}x/header.jpg"),
            &format!("{hash}/../header.jpg"),
            "//evil.example/header.jpg",
            &format!("https://evil.example/{hash}/header.jpg"),
            &format!("{hash}/header.jpg/extra"),
        ] {
            assert!(!is_valid_banner_path(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn a_hostile_banner_path_falls_back_to_the_classic_address() {
        let g = Game {
            app_id: 5,
            name: "x".into(),
            banner: Some("../../evil.jpg".into()),
        };
        assert_eq!(
            g.banner_url(),
            "https://cdn.cloudflare.steamstatic.com/steam/apps/5/header.jpg"
        );
    }
}
