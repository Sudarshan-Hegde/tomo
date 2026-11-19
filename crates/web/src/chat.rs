//! Real-time chat client: one WebSocket per signed-in session, with automatic reconnect.
//! Also owns the shared list of matches (with unread counts) that several pages display.

use std::rc::Rc;
use std::time::Duration;

use futures::{channel::mpsc, SinkExt, StreamExt};
use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use tomo_shared::{ClientMsg, MatchSummary, ServerMsg, Uuid};

use crate::api::{self, Session, SessionSignal};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChatStatus {
    Offline,
    Connecting,
    Online,
}

type Listener = Rc<dyn Fn(&ServerMsg)>;

/// A cheap, copyable handle. Provided through context from `App`.
#[derive(Clone, Copy)]
pub struct ChatClient {
    pub status: RwSignal<ChatStatus>,
    pub matches: RwSignal<Vec<MatchSummary>>,
    /// True once the matches list has been fetched at least once for this session.
    pub loaded: RwSignal<bool>,
    /// Total unread messages across all matches.
    pub unread: Memo<i64>,
    /// Bumped whenever the session changes, so a stale connection loop can stop itself.
    generation: StoredValue<u64>,
    out: StoredValue<Option<mpsc::UnboundedSender<ClientMsg>>, LocalStorage>,
    listeners: StoredValue<Vec<(u64, Listener)>, LocalStorage>,
    next_listener: StoredValue<u64>,
    refresh_pending: StoredValue<bool>,
}

impl ChatClient {
    pub fn new(session: SessionSignal) -> Self {
        let matches = RwSignal::new(Vec::<MatchSummary>::new());
        let client = ChatClient {
            status: RwSignal::new(ChatStatus::Offline),
            matches,
            loaded: RwSignal::new(false),
            unread: Memo::new(move |_| matches.with(|m| m.iter().map(|x| x.unread).sum())),
            generation: StoredValue::new(0),
            out: StoredValue::new_local(None),
            listeners: StoredValue::new_local(Vec::new()),
            next_listener: StoredValue::new(0),
            refresh_pending: StoredValue::new(false),
        };

        // (Re)connect whenever the signed-in user changes.
        Effect::new(move |_| {
            let current = session.get();
            client.generation.update_value(|g| *g += 1);
            let generation = client.generation.get_value();
            client.out.set_value(None);
            client.matches.set(Vec::new());
            client.loaded.set(false);
            match current {
                Some(s) => spawn_local(run(client, session, s, generation)),
                None => client.status.set(ChatStatus::Offline),
            }
        });
        client
    }

    /// Queue a frame for the server. Returns false if there is no open connection.
    pub fn send(&self, msg: ClientMsg) -> bool {
        self.out.with_value(|o| o.as_ref().map(|tx| tx.unbounded_send(msg).is_ok()).unwrap_or(false))
    }

    /// Listen for server events. Remember to `unsubscribe` in `on_cleanup`.
    pub fn subscribe(&self, f: impl Fn(&ServerMsg) + 'static) -> u64 {
        let id = self
            .next_listener
            .try_update_value(|n| {
                *n += 1;
                *n
            })
            .unwrap_or(0);
        self.listeners.update_value(|v| v.push((id, Rc::new(f))));
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        self.listeners.update_value(|v| v.retain(|(i, _)| *i != id));
    }

    fn dispatch(&self, msg: &ServerMsg) {
        let listeners: Vec<Listener> = self.listeners.with_value(|v| v.iter().map(|(_, f)| f.clone()).collect());
        for f in listeners {
            f(msg);
        }
    }

    /// Refetch the matches list. Bursts of events collapse into one request.
    pub fn refresh(&self, session: SessionSignal) {
        if self.refresh_pending.get_value() {
            return;
        }
        self.refresh_pending.set_value(true);
        let client = *self;
        spawn_local(async move {
            TimeoutFuture::new(150).await;
            client.refresh_pending.set_value(false);
            let Some(s) = session.get_untracked() else { return };
            match api::get::<Vec<MatchSummary>>("/matches", Some(&s.token)).await {
                Ok(v) => {
                    client.matches.set(v);
                    client.loaded.set(true);
                }
                Err(e) => {
                    api::handle_err(session, e);
                }
            }
        });
    }

    /// Optimistically zero the unread badge once the user is looking at the chat.
    pub fn clear_unread(&self, match_id: Uuid) {
        self.matches.update(|v| {
            if let Some(m) = v.iter_mut().find(|m| m.match_id == match_id) {
                m.unread = 0;
            }
        });
    }
}

/// Connection loop: connect, authenticate, pump events, and reconnect with backoff.
async fn run(client: ChatClient, session: SessionSignal, s: Session, generation: u64) {
    let current = move || client.generation.get_value() == generation;
    let mut backoff_ms: u32 = 1000;

    while current() {
        client.status.set(ChatStatus::Connecting);

        if let Ok(ws) = WebSocket::open(&api::ws_url()) {
            let (mut write, mut read) = ws.split();
            // First frame authenticates. The token is never put in the URL.
            let auth = serde_json::to_string(&ClientMsg::Auth { token: s.token.clone() }).unwrap_or_default();
            let _ = write.send(Message::Text(auth)).await;

            let (tx, mut rx) = mpsc::unbounded::<ClientMsg>();
            if current() {
                client.out.set_value(Some(tx));
            }
            spawn_local(async move {
                while let Some(msg) = rx.next().await {
                    let Ok(json) = serde_json::to_string(&msg) else { continue };
                    if write.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
            });

            while let Some(frame) = read.next().await {
                if !current() {
                    break;
                }
                let Ok(Message::Text(text)) = frame else { continue };
                let Ok(msg) = serde_json::from_str::<ServerMsg>(&text) else { continue };
                match &msg {
                    ServerMsg::Ready { .. } => {
                        client.status.set(ChatStatus::Online);
                        backoff_ms = 1000;
                        client.refresh(session); // catch up on anything missed while offline
                    }
                    ServerMsg::Unauthorized => {
                        api::handle_err(session, api::ApiErr::Unauthorized);
                        return;
                    }
                    ServerMsg::Message { .. } | ServerMsg::Read { .. } => client.refresh(session),
                    _ => {}
                }
                client.dispatch(&msg);
            }
        }

        if !current() {
            return;
        }
        client.out.set_value(None);
        client.status.set(ChatStatus::Offline);
        TimeoutFuture::new(backoff_ms).await;
        backoff_ms = (backoff_ms * 2).min(15_000);
    }
}

// ───────── small helpers shared by the chat UI ─────────

/// "14:05" in the viewer's local time.
pub fn fmt_time(ms: i64) -> String {
    let d = js_sys::Date::new(&leptos::wasm_bindgen::JsValue::from_f64(ms as f64));
    format!("{:02}:{:02}", d.get_hours(), d.get_minutes())
}

pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

pub const TYPING_IDLE: Duration = Duration::from_millis(3000);
pub const TYPING_RESEND_MS: f64 = 2500.0;
pub const TYPING_DISPLAY: Duration = Duration::from_millis(5000);
