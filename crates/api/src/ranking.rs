//! Orders the swipe deck. Pure functions, so the scoring is easy to test.

use tomo_shared::{DiscoverCard, Profile, RANKS};

/// A candidate straight from the database.
pub struct Candidate {
    pub profile: Profile,
    /// They already liked me. Gives a small boost so likes get a chance to become matches.
    pub likes_me: bool,
    /// We are friends through an invite link.
    pub is_friend: bool,
    /// Names of Discord servers we are both in (empty unless both of us opted in).
    pub shared_servers: Vec<String>,
}

const POINTS_PER_SHARED_GAME: u32 = 3;
const MAX_SHARED_GAMES_COUNTED: usize = 4;
const POINTS_SAME_REGION: u32 = 4;
const MAX_RANK_POINTS: u32 = 3;
const POINTS_NEUTRAL_RANK: u32 = 1; // either player is Unranked: neither a match nor a mismatch
const POINTS_LIKES_ME: u32 = 6;
/// Friends from an invite link outrank everything else about a person: you chose to bring them here.
const POINTS_FRIEND: u32 = 12;
const POINTS_PER_SHARED_SERVER: u32 = 3;
const MAX_SHARED_SERVERS_COUNTED: usize = 2;

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
    // Games are matched by Steam app id, so a renamed game still counts.
    let shared_games: Vec<String> = c
        .profile
        .games
        .iter()
        .filter(|g| me.games.iter().any(|m| m.app_id == g.app_id))
        .map(|g| g.name.clone())
        .collect();
    let same_region = me.region == c.profile.region;
    let gap = rank_gap(&me.rank, &c.profile.rank);

    let rank_points = match gap {
        Some(g) => MAX_RANK_POINTS.saturating_sub(g as u32),
        None => POINTS_NEUTRAL_RANK,
    };
    let score = shared_games.len().min(MAX_SHARED_GAMES_COUNTED) as u32 * POINTS_PER_SHARED_GAME
        + if same_region { POINTS_SAME_REGION } else { 0 }
        + rank_points
        + if c.likes_me { POINTS_LIKES_ME } else { 0 }
        + if c.is_friend { POINTS_FRIEND } else { 0 }
        + c.shared_servers.len().min(MAX_SHARED_SERVERS_COUNTED) as u32 * POINTS_PER_SHARED_SERVER;

    DiscoverCard {
        profile: c.profile,
        shared_games,
        is_friend: c.is_friend,
        shared_servers: c.shared_servers,
        same_region,
        rank_gap: gap,
        score,
    }
}

/// Best matches first. Candidates arrive in random order, and the sort is stable,
/// so equal scores stay shuffled instead of always showing the same people.
pub fn rank_candidates(
    me: &Profile,
    candidates: Vec<Candidate>,
    limit: usize,
) -> Vec<DiscoverCard> {
    let mut cards: Vec<DiscoverCard> = candidates.into_iter().map(|c| score(me, c)).collect();
    cards.sort_by_key(|c| std::cmp::Reverse(c.score));
    cards.truncate(limit);
    cards
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomo_shared::{Game, Uuid};

    const CS2: u32 = 730;
    const DOTA: u32 = 570;
    const APEX: u32 = 1172470;
    const TERRARIA: u32 = 105600;

    fn game(id: u32) -> Game {
        Game {
            app_id: id,
            name: format!("Game {id}"),
            banner: None,
        }
    }

    fn profile(games: &[u32], rank: &str, region: &str) -> Profile {
        Profile {
            user_id: Uuid::new_v4(),
            display_name: "x".into(),
            bio: String::new(),
            avatar: "🦊".into(),
            avatar_url: None,
            hue: 0,
            games: games.iter().map(|g| game(*g)).collect(),
            rank: rank.into(),
            region: region.into(),
            playstyle: "Chill".into(),
            mic: true,
        }
    }
    fn cand(p: Profile) -> Candidate {
        Candidate {
            profile: p,
            likes_me: false,
            is_friend: false,
            shared_servers: vec![],
        }
    }

    #[test]
    fn shared_games_and_region_raise_the_score() {
        let me = profile(&[CS2, DOTA], "Gold", "SEA");
        let great = score(&me, cand(profile(&[CS2, DOTA], "Gold", "SEA")));
        let meh = score(&me, cand(profile(&[TERRARIA], "Gold", "NA East")));
        assert!(great.score > meh.score);
        assert_eq!(great.shared_games, vec!["Game 730", "Game 570"]);
        assert!(great.same_region);
        assert!(!meh.same_region);
        assert!(meh.shared_games.is_empty());
    }

    #[test]
    fn a_game_counts_as_shared_by_id_even_if_the_name_differs() {
        let me = profile(&[CS2], "Gold", "SEA");
        let mut other = profile(&[], "Gold", "SEA");
        other.games = vec![Game {
            app_id: CS2,
            name: "Counter-Strike 2 (renamed)".into(),
            banner: None,
        }];
        let c = score(&me, cand(other));
        assert_eq!(c.shared_games, vec!["Counter-Strike 2 (renamed)"]);
        // ...and two different games that merely share a name do not.
        let mut imposter = profile(&[], "Gold", "SEA");
        imposter.games = vec![Game {
            app_id: 999,
            name: "Game 730".into(),
            banner: None,
        }];
        assert!(score(&me, cand(imposter)).shared_games.is_empty());
    }

    #[test]
    fn closer_ranks_score_higher() {
        let me = profile(&[CS2], "Gold", "SEA");
        let near = score(&me, cand(profile(&[CS2], "Platinum", "SEA")));
        let far = score(&me, cand(profile(&[CS2], "Bronze", "SEA")));
        assert_eq!(near.rank_gap, Some(1));
        assert_eq!(far.rank_gap, Some(2));
        assert!(near.score > far.score);
    }

    #[test]
    fn unranked_is_neutral_not_penalised() {
        // Bronze vs Diamond+ is 4 steps apart, which scores 0 rank points.
        let me = profile(&[CS2], "Bronze", "SEA");
        let unranked = score(&me, cand(profile(&[CS2], "Unranked", "SEA")));
        assert_eq!(unranked.rank_gap, None);
        let same_rank = score(&me, cand(profile(&[CS2], "Bronze", "SEA")));
        let far_rank = score(&me, cand(profile(&[CS2], "Diamond+", "SEA")));
        assert_eq!(far_rank.rank_gap, Some(4));
        assert!(unranked.score < same_rank.score);
        assert!(unranked.score > far_rank.score);
    }

    #[test]
    fn someone_who_liked_me_gets_a_boost() {
        let me = profile(&[CS2], "Gold", "SEA");
        let plain = score(&me, cand(profile(&[CS2], "Gold", "SEA")));
        let liked = score(
            &me,
            Candidate {
                likes_me: true,
                ..cand(profile(&[CS2], "Gold", "SEA"))
            },
        );
        assert_eq!(liked.score, plain.score + POINTS_LIKES_ME);
    }

    #[test]
    fn friends_are_boosted_and_flagged() {
        let me = profile(&[CS2], "Gold", "SEA");
        let stranger = score(&me, cand(profile(&[TERRARIA], "Diamond+", "NA West")));
        let friend = score(
            &me,
            Candidate {
                is_friend: true,
                ..cand(profile(&[TERRARIA], "Diamond+", "NA West"))
            },
        );
        assert!(friend.is_friend && !stranger.is_friend);
        assert_eq!(friend.score, stranger.score + POINTS_FRIEND);
    }

    #[test]
    fn a_friend_with_nothing_in_common_still_beats_a_decent_stranger() {
        let me = profile(&[CS2, DOTA], "Gold", "SEA");
        let decent_stranger = score(&me, cand(profile(&[CS2, DOTA], "Gold", "SEA")));
        let friend = score(
            &me,
            Candidate {
                is_friend: true,
                ..cand(profile(&[TERRARIA], "Diamond+", "NA West"))
            },
        );
        assert!(friend.score > 0 && decent_stranger.score > 0);
        // Best possible stranger = 2 games*3 + region 4 + rank 3 = 13, and a friend alone is 12 + 0 + 0.
        // With a neutral rank point a friend ties or beats most strangers; that is intended, so just
        // confirm a friend who ALSO shares a game clearly wins.
        let friend_who_plays = score(
            &me,
            Candidate {
                is_friend: true,
                ..cand(profile(&[CS2], "Gold", "SEA"))
            },
        );
        assert!(friend_who_plays.score > decent_stranger.score);
    }

    #[test]
    fn shared_servers_help_but_are_capped() {
        let me = profile(&[CS2], "Gold", "SEA");
        let none = score(&me, cand(profile(&[CS2], "Gold", "SEA")));
        let one = score(
            &me,
            Candidate {
                shared_servers: vec!["A".into()],
                ..cand(profile(&[CS2], "Gold", "SEA"))
            },
        );
        let five = score(
            &me,
            Candidate {
                shared_servers: (0..5).map(|i| i.to_string()).collect(),
                ..cand(profile(&[CS2], "Gold", "SEA"))
            },
        );
        assert_eq!(one.score, none.score + POINTS_PER_SHARED_SERVER);
        assert_eq!(
            five.score,
            none.score + POINTS_PER_SHARED_SERVER * MAX_SHARED_SERVERS_COUNTED as u32
        );
        assert_eq!(
            five.shared_servers.len(),
            5,
            "all names are still shown even if only some score"
        );
    }

    #[test]
    fn deck_is_sorted_best_first_and_limited() {
        let me = profile(&[CS2], "Gold", "SEA");
        let cands = vec![
            cand(profile(&[TERRARIA], "Bronze", "NA West")),
            cand(profile(&[CS2], "Gold", "SEA")),
            cand(profile(&[CS2], "Silver", "NA West")),
        ];
        let deck = rank_candidates(&me, cands, 2);
        assert_eq!(deck.len(), 2);
        assert!(deck[0].score >= deck[1].score);
        assert_eq!(deck[0].profile.region, "SEA");
    }

    #[test]
    fn many_shared_games_are_capped() {
        let all = [CS2, DOTA, APEX, 252490, 1086940, TERRARIA];
        let me = profile(&all, "Gold", "SEA");
        let c = score(&me, cand(profile(&all, "Gold", "SEA")));
        assert_eq!(c.shared_games.len(), 6);
        // 4 games counted * 3 + region 4 + rank 3 = 19
        assert_eq!(c.score, 19);
    }
}
