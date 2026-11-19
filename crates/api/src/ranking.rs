//! Orders the swipe deck. Pure functions, so the scoring is easy to test.

use tomo_shared::{DiscoverCard, Profile, RANKS};

/// A candidate straight from the database.
pub struct Candidate {
    pub profile: Profile,
    /// They already liked me. Gives a small boost so likes get a chance to become matches.
    pub likes_me: bool,
}

const POINTS_PER_SHARED_GAME: u32 = 3;
const MAX_SHARED_GAMES_COUNTED: usize = 4;
const POINTS_SAME_REGION: u32 = 4;
const MAX_RANK_POINTS: u32 = 3;
const POINTS_NEUTRAL_RANK: u32 = 1; // either player is Unranked: neither a match nor a mismatch
const POINTS_LIKES_ME: u32 = 6;

fn rank_index(rank: &str) -> Option<usize> {
    RANKS.iter().position(|r| *r == rank)
}

/// Steps between two ranks, or None if either is Unranked (index 0) or unknown.
fn rank_gap(a: &str, b: &str) -> Option<u8> {
    match (rank_index(a), rank_index(b)) {
        (Some(x), Some(y)) if x != 0 && y != 0 => Some(x.abs_diff(y) as u8),
        _ => None,
    }
}

pub fn score(me: &Profile, c: Candidate) -> DiscoverCard {
    let shared_games: Vec<String> = c.profile.games.iter().filter(|g| me.games.contains(g)).cloned().collect();
    let same_region = me.region == c.profile.region;
    let gap = rank_gap(&me.rank, &c.profile.rank);

    let rank_points = match gap {
        Some(g) => MAX_RANK_POINTS.saturating_sub(g as u32),
        None => POINTS_NEUTRAL_RANK,
    };
    let score = shared_games.len().min(MAX_SHARED_GAMES_COUNTED) as u32 * POINTS_PER_SHARED_GAME
        + if same_region { POINTS_SAME_REGION } else { 0 }
        + rank_points
        + if c.likes_me { POINTS_LIKES_ME } else { 0 };

    DiscoverCard { profile: c.profile, shared_games, same_region, rank_gap: gap, score }
}

/// Best matches first. Candidates arrive in random order, and the sort is stable,
/// so equal scores stay shuffled instead of always showing the same people.
pub fn rank_candidates(me: &Profile, candidates: Vec<Candidate>, limit: usize) -> Vec<DiscoverCard> {
    let mut cards: Vec<DiscoverCard> = candidates.into_iter().map(|c| score(me, c)).collect();
    cards.sort_by(|a, b| b.score.cmp(&a.score));
    cards.truncate(limit);
    cards
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomo_shared::Uuid;

    fn profile(games: &[&str], rank: &str, region: &str) -> Profile {
        Profile {
            user_id: Uuid::new_v4(),
            display_name: "x".into(),
            bio: String::new(),
            avatar: "🦊".into(),
            hue: 0,
            games: games.iter().map(|g| g.to_string()).collect(),
            rank: rank.into(),
            region: region.into(),
            playstyle: "Chill".into(),
            mic: true,
        }
    }
    fn cand(p: Profile) -> Candidate {
        Candidate { profile: p, likes_me: false }
    }

    #[test]
    fn shared_games_and_region_raise_the_score() {
        let me = profile(&["Valorant", "CS2"], "Gold", "SEA");
        let great = score(&me, cand(profile(&["Valorant", "CS2"], "Gold", "SEA")));
        let meh = score(&me, cand(profile(&["Minecraft"], "Gold", "NA East")));
        assert!(great.score > meh.score);
        assert_eq!(great.shared_games, vec!["Valorant", "CS2"]);
        assert!(great.same_region);
        assert!(!meh.same_region);
        assert!(meh.shared_games.is_empty());
    }

    #[test]
    fn closer_ranks_score_higher() {
        let me = profile(&["Valorant"], "Gold", "SEA");
        let near = score(&me, cand(profile(&["Valorant"], "Platinum", "SEA")));
        let far = score(&me, cand(profile(&["Valorant"], "Bronze", "SEA")));
        assert_eq!(near.rank_gap, Some(1));
        assert_eq!(far.rank_gap, Some(2));
        assert!(near.score > far.score);
    }

    #[test]
    fn unranked_is_neutral_not_penalised() {
        // Bronze vs Diamond+ is 4 steps apart, which scores 0 rank points.
        let me = profile(&["Valorant"], "Bronze", "SEA");
        let unranked = score(&me, cand(profile(&["Valorant"], "Unranked", "SEA")));
        assert_eq!(unranked.rank_gap, None);
        let same_rank = score(&me, cand(profile(&["Valorant"], "Bronze", "SEA")));
        let far_rank = score(&me, cand(profile(&["Valorant"], "Diamond+", "SEA")));
        assert_eq!(far_rank.rank_gap, Some(4));
        assert!(unranked.score < same_rank.score);
        assert!(unranked.score > far_rank.score);
    }

    #[test]
    fn someone_who_liked_me_gets_a_boost() {
        let me = profile(&["Valorant"], "Gold", "SEA");
        let plain = score(&me, Candidate { profile: profile(&["Valorant"], "Gold", "SEA"), likes_me: false });
        let liked = score(&me, Candidate { profile: profile(&["Valorant"], "Gold", "SEA"), likes_me: true });
        assert_eq!(liked.score, plain.score + POINTS_LIKES_ME);
    }

    #[test]
    fn deck_is_sorted_best_first_and_limited() {
        let me = profile(&["Valorant"], "Gold", "SEA");
        let cands = vec![
            cand(profile(&["Minecraft"], "Bronze", "NA West")),
            cand(profile(&["Valorant"], "Gold", "SEA")),
            cand(profile(&["Valorant"], "Silver", "NA West")),
        ];
        let deck = rank_candidates(&me, cands, 2);
        assert_eq!(deck.len(), 2);
        assert!(deck[0].score >= deck[1].score);
        assert_eq!(deck[0].profile.region, "SEA");
    }

    #[test]
    fn many_shared_games_are_capped() {
        let all = ["Valorant", "CS2", "League of Legends", "Apex Legends", "Overwatch 2", "Dota 2"];
        let me = profile(&all, "Gold", "SEA");
        let c = score(&me, cand(profile(&all, "Gold", "SEA")));
        assert_eq!(c.shared_games.len(), 6);
        // 4 games counted * 3 + region 4 + rank 3 = 19
        assert_eq!(c.score, 19);
    }
}
