use serde::{Deserialize, Serialize};
pub use uuid::Uuid;

// ───────── Allowed values: single source of truth for UI and API validation ─────────

pub const GAMES: [&str; 10] = [
    "Valorant",
    "CS2",
    "League of Legends",
    "Apex Legends",
    "Overwatch 2",
    "Dota 2",
    "Rocket League",
    "Minecraft",
    "Fortnite",
    "Genshin Impact",
];
pub const RANKS: [&str; 6] = ["Unranked", "Bronze", "Silver", "Gold", "Platinum", "Diamond+"];
pub const PLAYSTYLES: [&str; 5] = ["Chill", "Competitive", "Support main", "Entry fragger", "Strategist"];
pub const REGIONS: [&str; 5] = ["India (IST)", "Europe (CET)", "NA East", "NA West", "SEA"];
pub const AVATARS: [&str; 12] = ["🦊", "👹", "🥷", "👻", "👺", "🌑", "⚡", "🌙", "🐺", "🦇", "🗡️", "🏯"];

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
    /// 0-360, tints the avatar gradient.
    pub hue: u16,
    pub games: Vec<String>,
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
    pub games: Vec<String>,
    pub rank: String,
    pub region: String,
    pub playstyle: String,
    pub mic: bool,
    /// "I am 18 or older". Required the first time a profile is created.
    pub adult_confirmed: bool,
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
        if self.games.is_empty() || self.games.len() > GAMES.len() {
            return Err("Pick at least one game.".into());
        }
        if self.games.iter().any(|g| !GAMES.contains(&g.as_str())) {
            return Err("Unknown game.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevLoginRequest {
    pub username: String,
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
}

/// A candidate on the swipe deck, with the reasons it was suggested.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoverCard {
    pub profile: Profile,
    pub shared_games: Vec<String>,
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
    Auth { token: String },
    Send { match_id: Uuid, body: String },
    Typing { match_id: Uuid, typing: bool },
    /// I have read everything up to and including this message id.
    Read { match_id: Uuid, up_to: i64 },
}

/// Frames sent by the server over the WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Ready { user_id: Uuid },
    Message { message: ChatMessage },
    Typing { match_id: Uuid, user_id: Uuid, typing: bool },
    Read { match_id: Uuid, user_id: Uuid, up_to: i64 },
    Error { message: String },
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
            games: vec!["Valorant".into()],
            rank: "Gold".into(),
            region: "SEA".into(),
            playstyle: "Chill".into(),
            mic: true,
            adult_confirmed: true,
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
        p.games = vec!["Not A Game".into()];
        assert!(p.validate().is_err());

        let mut p = valid();
        p.rank = "Mythic".into();
        assert!(p.validate().is_err());

        let mut p = valid();
        p.bio = "x".repeat(281);
        assert!(p.validate().is_err());
    }
}
