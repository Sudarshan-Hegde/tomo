//! Matches list, message history, and the real-time WebSocket.

use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    response::Response,
    Json,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use sqlx::FromRow;
use tomo_shared::{ChatHistory, ChatMessage, ClientMsg, MatchSummary, ServerMsg, MAX_MESSAGE_LEN};
use uuid::Uuid;

use crate::{
    auth::{user_for_token, AuthUser},
    error::ApiError,
    routes::ProfileRow,
    safety::is_blocked,
    AppState,
};

const HISTORY_PAGE: i64 = 50;
const SEND_LIMIT: usize = 20;
const SEND_WINDOW: Duration = Duration::from_secs(10);
/// How long a new connection has to send its auth frame.
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for the database to answer the token lookup (it may be waking from sleep).
const DB_AUTH_TIMEOUT: Duration = Duration::from_secs(20);

fn to_message(id: i64, match_id: Uuid, sender_id: Uuid, body: String, at: chrono::DateTime<chrono::Utc>) -> ChatMessage {
    ChatMessage { id, match_id, sender_id, body, created_at_ms: at.timestamp_millis() }
}

/// If `user` is part of the match, returns the other participant.
async fn other_participant(db: &sqlx::PgPool, match_id: Uuid, user: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    let pair = sqlx::query_as::<_, (Uuid, Uuid)>("SELECT user_a, user_b FROM matches WHERE id = $1")
        .bind(match_id)
        .fetch_optional(db)
        .await?;
    Ok(pair.and_then(|(a, b)| {
        if a == user {
            Some(b)
        } else if b == user {
            Some(a)
        } else {
            None
        }
    }))
}

// ───────── REST ─────────

#[derive(FromRow)]
struct MatchRow {
    match_id: Uuid,
    #[sqlx(flatten)]
    profile: ProfileRow,
    lm_id: Option<i64>,
    lm_sender: Option<Uuid>,
    lm_body: Option<String>,
    lm_at: Option<chrono::DateTime<chrono::Utc>>,
    unread: i64,
}

pub async fn matches(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
) -> Result<Json<Vec<MatchSummary>>, ApiError> {
    let rows = sqlx::query_as::<_, MatchRow>(concat!(
        "SELECT m.id AS match_id, ",
        profile_cols_p!(),
        ",
            lm.id AS lm_id, lm.sender_id AS lm_sender, lm.body AS lm_body, lm.created_at AS lm_at,
            (SELECT count(*) FROM messages x
              WHERE x.match_id = m.id AND x.sender_id <> $1 AND x.id > COALESCE(r.last_read_id, 0)) AS unread
         FROM matches m
         JOIN profiles p ON p.user_id = CASE WHEN m.user_a = $1 THEN m.user_b ELSE m.user_a END
         LEFT JOIN match_reads r ON r.match_id = m.id AND r.user_id = $1
         LEFT JOIN LATERAL (SELECT id, sender_id, body, created_at FROM messages
                            WHERE match_id = m.id ORDER BY id DESC LIMIT 1) lm ON true
         WHERE (m.user_a = $1 OR m.user_b = $1)
           AND NOT EXISTS (SELECT 1 FROM blocks b
                           WHERE (b.blocker_id = $1 AND b.blocked_id = p.user_id)
                              OR (b.blocker_id = p.user_id AND b.blocked_id = $1))
         ORDER BY COALESCE(lm.created_at, m.created_at) DESC"
    ))
    .bind(uid)
    .fetch_all(&state.db)
    .await?;

    let list = rows
        .into_iter()
        .map(|r| {
            let last_message = match (r.lm_id, r.lm_sender, r.lm_body, r.lm_at) {
                (Some(id), Some(sender), Some(body), Some(at)) => Some(to_message(id, r.match_id, sender, body, at)),
                _ => None,
            };
            MatchSummary { match_id: r.match_id, profile: r.profile.into(), last_message, unread: r.unread }
        })
        .collect();
    Ok(Json(list))
}

#[derive(Deserialize)]
pub struct HistoryParams {
    /// Only messages older than this id (for loading earlier history).
    before: Option<i64>,
    limit: Option<i64>,
}

pub async fn history(
    State(state): State<AppState>,
    AuthUser(uid): AuthUser,
    Path(match_id): Path<Uuid>,
    Query(params): Query<HistoryParams>,
) -> Result<Json<ChatHistory>, ApiError> {
    let other = other_participant(&state.db, match_id, uid)
        .await?
        .ok_or(ApiError::NotFound("You have no pact with this player."))?;
    if is_blocked(&state.db, uid, other).await? {
        return Err(ApiError::NotFound("You have no pact with this player."));
    }

    let limit = params.limit.unwrap_or(HISTORY_PAGE).clamp(1, 100);
    // Fetch one extra row to learn whether older messages exist.
    let mut rows = sqlx::query_as::<_, (i64, Uuid, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, sender_id, body, created_at FROM messages
         WHERE match_id = $1 AND ($2::bigint IS NULL OR id < $2)
         ORDER BY id DESC LIMIT $3",
    )
    .bind(match_id)
    .bind(params.before)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await?;

    let has_more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    rows.reverse(); // oldest first
    let messages = rows
        .into_iter()
        .map(|(id, sender, body, at)| to_message(id, match_id, sender, body, at))
        .collect();

    let other_last_read = sqlx::query_scalar::<_, i64>(
        "SELECT last_read_id FROM match_reads WHERE match_id = $1 AND user_id = $2",
    )
    .bind(match_id)
    .bind(other)
    .fetch_optional(&state.db)
    .await?
    .unwrap_or(0);

    Ok(Json(ChatHistory { messages, other_last_read, has_more }))
}

// ───────── WebSocket ─────────

pub async fn ws_handler(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(16 * 1024).on_upgrade(move |socket| handle_socket(socket, state))
}

async fn send_frame(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, msg: &ServerMsg) -> bool {
    match serde_json::to_string(msg) {
        Ok(json) => sink.send(Message::Text(json.into())).await.is_ok(),
        Err(_) => false,
    }
}

/// Per-connection state.
struct Conn {
    user: Uuid,
    /// match id -> the other person, cached after the first participant check.
    others: HashMap<Uuid, Uuid>,
    recent_sends: VecDeque<Instant>,
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();

    // The first frame must be {"type":"auth","token":"..."}. Doing it here (not in the URL)
    // keeps tokens out of access logs and browser history.
    let first_frame = tokio::time::timeout(AUTH_TIMEOUT, async {
        while let Some(Ok(frame)) = stream.next().await {
            if let Message::Text(text) = frame {
                return Some(text);
            }
        }
        None
    })
    .await;

    let token = match first_frame.ok().flatten().and_then(|t| serde_json::from_str::<ClientMsg>(&t).ok()) {
        Some(ClientMsg::Auth { token }) => token,
        _ => {
            // No usable auth frame: this client is not who it claims to be.
            let _ = send_frame(&mut sink, &ServerMsg::Unauthorized).await;
            return;
        }
    };

    // "Unauthorized" must only ever mean "your token is not valid". A database hiccup (for example
    // a free-tier database waking up, or the machine resuming from sleep) is NOT that, because the
    // browser reacts to Unauthorized by deleting the saved login. For temporary trouble we just
    // close the socket, and the browser retries.
    let lookup = tokio::time::timeout(DB_AUTH_TIMEOUT, user_for_token(&state.db, &token)).await;
    let user = match lookup {
        Ok(Ok(Some(user))) => user,
        Ok(Ok(None)) => {
            let _ = send_frame(&mut sink, &ServerMsg::Unauthorized).await;
            return;
        }
        Ok(Err(e)) => {
            tracing::warn!("websocket auth: database error, asking client to retry: {e}");
            let _ = send_frame(&mut sink, &ServerMsg::Error { message: "The server is waking up. Retrying…".into() }).await;
            return;
        }
        Err(_) => {
            tracing::warn!("websocket auth: database lookup timed out, asking client to retry");
            let _ = send_frame(&mut sink, &ServerMsg::Error { message: "The server is waking up. Retrying…".into() }).await;
            return;
        }
    };

    let (conn_id, own_tx, mut rx) = state.hub.register(user);
    let writer = tokio::spawn(async move {
        let _ = send_frame(&mut sink, &ServerMsg::Ready { user_id: user }).await;
        while let Some(msg) = rx.recv().await {
            if !send_frame(&mut sink, &msg).await {
                break;
            }
        }
    });

    let mut conn = Conn { user, others: HashMap::new(), recent_sends: VecDeque::new() };
    while let Some(Ok(frame)) = stream.next().await {
        match frame {
            Message::Text(text) => match serde_json::from_str::<ClientMsg>(&text) {
                Ok(ClientMsg::Auth { .. }) => {}
                Ok(msg) => {
                    if let Err(message) = handle_client_msg(&state, &mut conn, msg).await {
                        let _ = own_tx.send(ServerMsg::Error { message });
                    }
                }
                Err(_) => {
                    let _ = own_tx.send(ServerMsg::Error { message: "Unrecognised message.".into() });
                }
            },
            Message::Close(_) => break,
            _ => {}
        }
    }

    state.hub.unregister(user, conn_id);
    writer.abort();
}

/// Sliding-window rate limit: at most SEND_LIMIT messages per SEND_WINDOW per connection.
/// Records the send and returns true if allowed.
fn allow_send(recent: &mut VecDeque<Instant>, now: Instant) -> bool {
    while recent.front().is_some_and(|t| now.duration_since(*t) > SEND_WINDOW) {
        recent.pop_front();
    }
    if recent.len() >= SEND_LIMIT {
        return false;
    }
    recent.push_back(now);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_the_limit_then_blocks() {
        let mut recent = VecDeque::new();
        let now = Instant::now();
        for i in 0..SEND_LIMIT {
            assert!(allow_send(&mut recent, now), "send {i} should be allowed");
        }
        assert!(!allow_send(&mut recent, now), "send over the limit should be refused");
    }

    #[test]
    fn frees_up_once_the_window_passes() {
        let mut recent = VecDeque::new();
        let start = Instant::now();
        for _ in 0..SEND_LIMIT {
            assert!(allow_send(&mut recent, start));
        }
        assert!(!allow_send(&mut recent, start + Duration::from_secs(5)));
        assert!(allow_send(&mut recent, start + SEND_WINDOW + Duration::from_secs(1)));
    }

    #[test]
    fn refused_sends_are_not_counted() {
        let mut recent = VecDeque::new();
        let now = Instant::now();
        for _ in 0..(SEND_LIMIT + 10) {
            allow_send(&mut recent, now);
        }
        assert_eq!(recent.len(), SEND_LIMIT);
    }
}

/// Returns an error message to show to this connection only.
async fn handle_client_msg(state: &AppState, conn: &mut Conn, msg: ClientMsg) -> Result<(), String> {
    const GENERIC: &str = "Something went wrong.";
    match msg {
        ClientMsg::Auth { .. } => Ok(()),

        ClientMsg::Send { match_id, body } => {
            let body = body.trim().to_string();
            if body.is_empty() {
                return Ok(());
            }
            if body.chars().count() > MAX_MESSAGE_LEN {
                return Err(format!("Messages can be at most {MAX_MESSAGE_LEN} characters."));
            }

            if !allow_send(&mut conn.recent_sends, Instant::now()) {
                return Err("You are sending messages too quickly. Slow down.".into());
            }

            // One statement checks membership AND blocks AND inserts, so a block takes effect
            // immediately (no gap between check and insert) and a send costs one round trip.
            let inserted = sqlx::query_as::<_, (i64, chrono::DateTime<chrono::Utc>, Uuid)>(
                "INSERT INTO messages (match_id, sender_id, body)
                 SELECT m.id, $2, $3 FROM matches m
                  WHERE m.id = $1 AND (m.user_a = $2 OR m.user_b = $2)
                    AND NOT EXISTS (SELECT 1 FROM blocks b
                                    WHERE (b.blocker_id = m.user_a AND b.blocked_id = m.user_b)
                                       OR (b.blocker_id = m.user_b AND b.blocked_id = m.user_a))
                 RETURNING id, created_at, (SELECT CASE WHEN user_a = $2 THEN user_b ELSE user_a END
                                              FROM matches WHERE id = $1)",
            )
            .bind(match_id)
            .bind(conn.user)
            .bind(&body)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| {
                tracing::error!("insert message: {e}");
                GENERIC.to_string()
            })?;

            let Some((id, at, other)) = inserted else {
                // Rare path, so a second query to give the right explanation is fine.
                return Err(match participant(state, conn, match_id).await {
                    Ok(_) => "You can no longer message this player.".into(),
                    Err(msg) => msg,
                });
            };
            conn.others.insert(match_id, other);

            let event = ServerMsg::Message { message: to_message(id, match_id, conn.user, body, at) };
            state.hub.send_to(conn.user, &event); // echo to the sender (all their tabs)
            state.hub.send_to(other, &event);
            Ok(())
        }

        ClientMsg::Typing { match_id, typing } => {
            let other = participant(state, conn, match_id).await?;
            state.hub.send_to(other, &ServerMsg::Typing { match_id, user_id: conn.user, typing });
            Ok(())
        }

        ClientMsg::Read { match_id, up_to } => {
            let other = participant(state, conn, match_id).await?;
            // Clamp to the newest real message so nobody can mark the future as read.
            let up_to = sqlx::query_scalar::<_, i64>(
                "INSERT INTO match_reads (match_id, user_id, last_read_id)
                 VALUES ($1, $2, LEAST($3, (SELECT COALESCE(MAX(id), 0) FROM messages WHERE match_id = $1)))
                 ON CONFLICT (match_id, user_id) DO UPDATE
                    SET last_read_id = GREATEST(match_reads.last_read_id, EXCLUDED.last_read_id), updated_at = now()
                 RETURNING last_read_id",
            )
            .bind(match_id)
            .bind(conn.user)
            .bind(up_to)
            .fetch_one(&state.db)
            .await
            .map_err(|e| {
                tracing::error!("update read pointer: {e}");
                GENERIC.to_string()
            })?;

            let event = ServerMsg::Read { match_id, user_id: conn.user, up_to };
            state.hub.send_to(other, &event);
            state.hub.send_to(conn.user, &event); // keeps unread badges in sync across my tabs
            Ok(())
        }
    }
}

/// Confirms the connection's user belongs to this match; caches the answer.
async fn participant(state: &AppState, conn: &mut Conn, match_id: Uuid) -> Result<Uuid, String> {
    if let Some(other) = conn.others.get(&match_id) {
        return Ok(*other);
    }
    let other = other_participant(&state.db, match_id, conn.user)
        .await
        .map_err(|_| "Something went wrong.".to_string())?
        .ok_or_else(|| "You have no pact with this player.".to_string())?;
    conn.others.insert(match_id, other);
    Ok(other)
}
