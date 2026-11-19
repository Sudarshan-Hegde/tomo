use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};
use leptos_router::NavigateOptions;
use tomo_shared::{
    AuthConfig, AuthResponse, ChatHistory, ChatMessage, ClientMsg, DevLoginRequest, DiscordInfo,
    DiscoverCard, ExchangeRequest, MatchedWith, Me, Profile, ProfileInput, ServerMsg, SwipeRequest,
    SwipeResponse, Uuid, AVATARS, PLAYSTYLES, RANKS, REGIONS,
};

use crate::api::{self, ApiErr, NoticeSignal, SessionSignal};
use crate::chat::{
    fmt_time, now_ms, ChatClient, ChatStatus, TYPING_DISPLAY, TYPING_IDLE, TYPING_RESEND_MS,
};
use crate::components::{
    ApiStatus, Avatar, BlockDialog, GamePicker, ProfileCard, ReportDialog, SignInPrompt,
};
use tomo_shared::{
    AcceptInviteRequest, AcceptInviteResponse, Friend, Game, InviteInfo, InvitePreview,
    ServerPerson, ServersStatus,
};

// ───────────────────────── Landing ─────────────────────────

#[component]
pub fn Landing() -> impl IntoView {
    let features = [
        (
            "🎯",
            "Rank-bound clans",
            "Your deck is ranked by shared games, region and rank, so the best allies come first.",
            true,
        ),
        (
            "🌙",
            "Night-owl sync",
            "Match with players who are awake when you are.",
            false,
        ),
        (
            "🎙️",
            "Voice or silence",
            "War cry or silent blade. Say what you want up front.",
            true,
        ),
        (
            "📜",
            "Whispered scrolls",
            "Real-time chat with typing signs and seen marks once a pact is sealed.",
            true,
        ),
        (
            "🛡️",
            "Banish & report",
            "Block or report anyone. Everyone is 18+, confirmed at sign-up.",
            true,
        ),
        (
            "⚔️",
            "Raise a war band",
            "Fill a full 5-stack with one click.",
            false,
        ),
    ];
    view! {
        <section class="hero">
            <div class="deco moon"></div>
            <div class="deco hero-kanji" aria-hidden="true">"侍"</div>
            <div class="deco embers" aria-hidden="true">
                <i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i>
            </div>
            <p class="eyebrow">"友 · Tomo · bound by blade"</p>
            <h1>
                "No ronin fights alone. "
                <span class="grad">"Find your clan."</span>
            </h1>
            <p class="lead">
                "Swipe on gamers by rank, playstyle and timezone. Seal a pact, then take the lobby together."
            </p>
            <div class="cta">
                <A href="/discover" attr:class="btn primary big">"Draw your blade →"</A>
                <A href="/login" attr:class="btn ghost big">"Sign in"</A>
            </div>
        </section>

        <section class="section">
            <h2>"The way of the clan"</h2>
            <div class="steps">
                <div class="step"><span class="num">"1"</span><h4>"Forge your legend"</h4><p>"Pick your games, rank, region and temperament."</p></div>
                <div class="step"><span class="num">"2"</span><h4>"Choose your allies"</h4><p>"Seal the pact to recruit, or pass to walk on."</p></div>
                <div class="step"><span class="num">"3"</span><h4>"Take the lobby"</h4><p>"A mutual pact? Send a scroll and queue up."</p></div>
            </div>
        </section>

        <section class="section">
            <h2>"Weapons of the trade"</h2>
            <div class="grid">
                {features
                    .iter()
                    .map(|(icon, title, desc, live)| {
                        view! {
                            <div class="feature">
                                <div class="f-icon">{*icon}</div>
                                <h4>{*title} {(!*live).then(|| view! { <em class="soon">"soon"</em> })}</h4>
                                <p>{*desc}</p>
                            </div>
                        }
                    })
                    .collect_view()}
            </div>
        </section>

        <footer class="footer">
            <span>"Built with Rust 🦀 · Axum · Leptos · Neon"</span>
            <ApiStatus />
        </footer>
    }
}

// ───────────────────────── Login ─────────────────────────

/// Where to go after signing in: new accounts build a profile first, returning ones start hunting.
async fn landing_after_login(token: &str) -> &'static str {
    match api::get::<Me>("/me", Some(token)).await {
        Ok(Me {
            profile: Some(_), ..
        }) => "/discover",
        _ => "/welcome",
    }
}

/// If the person arrived through a friend's invite link before signing in, use it now.
async fn accept_pending_invite(token: &str, notice: NoticeSignal) {
    let Some(code) = api::take_pending_invite() else {
        return;
    };
    let req = AcceptInviteRequest { code };
    match api::post::<_, AcceptInviteResponse>("/invites/accept", Some(token), &req).await {
        Ok(r) if r.already_friends => notice.set(Some(format!(
            "You and {} are already friends.",
            r.friend_name
        ))),
        Ok(r) => notice.set(Some(format!("You and {} are now friends!", r.friend_name))),
        // For example "That is your own invite link". Worth showing, never worth blocking sign-in.
        Err(e) => notice.set(Some(e.text())),
    }
}

#[component]
pub fn Login() -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    let notice = expect_context::<NoticeSignal>();
    let servers = RwSignal::new(false); // "also find players from my Discord servers"
    let navigate = use_navigate();
    let query = use_query_map();

    // Which sign-in methods does this server offer?
    let methods =
        LocalResource::new(|| async { api::get::<AuthConfig>("/auth/config", None).await });

    let username = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    // The server sends people back here with ?error=... if Discord login failed or was cancelled.
    Effect::new(move |_| {
        if let Some(msg) = query.read().get("error") {
            error.set(Some(msg));
        }
    });

    // A Callback is Copy, so the reactive view below can hand it to handlers repeatedly.
    let submit = Callback::new(move |_: ()| {
        if busy.get_untracked() {
            return;
        }
        let name = username.get_untracked().trim().to_string();
        if name.is_empty() {
            error.set(Some("Choose a username.".into()));
            return;
        }
        busy.set(true);
        error.set(None);
        let navigate = navigate.clone();
        spawn_local(async move {
            let result = api::post::<_, AuthResponse>(
                "/auth/dev",
                None,
                &DevLoginRequest { username: name },
            )
            .await;
            match result {
                Ok(auth) => {
                    let s = api::Session {
                        token: auth.token,
                        user: auth.user,
                    };
                    api::store_session(Some(&s));
                    let target = landing_after_login(&s.token).await;
                    accept_pending_invite(&s.token, notice).await;
                    session.set(Some(s));
                    navigate(target, Default::default());
                }
                Err(e) => error.set(Some(e.text())),
            }
            busy.set(false);
        });
    });

    view! {
        <section class="center-card">
            <div class="brand-mark huge">"友"</div>
            <h2>"Enter the dojo"</h2>
            <p class="muted">"Sign in to save your profile and your pacts."</p>

            {move || error.get().map(|e| view! { <p class="error">{e}</p> })}

            {move || match methods.get() {
                None => view! { <p class="muted">"Checking the gate…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="error">{e.text()}</p> }.into_any(),
                Some(Ok(m)) => {
                    let discord = if m.discord {
                        // A normal link, not a fetch: the whole page goes to Discord and comes back.
                        view! {
                            <a
                                class="btn discord big"
                                href=move || {
                                    format!("{}/auth/discord{}", api::API_URL, if servers.get() { "?servers=1" } else { "" })
                                }
                            >
                                "Continue with Discord"
                            </a>
                            <label class="check servers-opt">
                                <input
                                    type="checkbox"
                                    prop:checked=move || servers.get()
                                    on:change:target=move |ev| servers.set(ev.target().checked())
                                />
                                <span>
                                    "Also find players from my Discord servers. This asks Discord for your server list (small communities only, and only the names of servers you share are ever shown). You can turn it off any time."
                                </span>
                            </label>
                        }
                            .into_any()
                    } else {
                        view! {
                            <button class="btn discord big" disabled=true title="Discord sign-in is not set up on this server">
                                "Continue with Discord"
                            </button>
                        }
                            .into_any()
                    };
                    let dev = m.dev.then(|| {
                        view! {
                            <div class="divider"><span>"development only"</span></div>
                            <form
                                class="form"
                                on:submit=move |ev| {
                                    ev.prevent_default();
                                    submit.run(());
                                }
                            >
                                <input
                                    type="text"
                                    placeholder="any username"
                                    maxlength="24"
                                    autocomplete="username"
                                    prop:value=move || username.get()
                                    on:input:target=move |ev| username.set(ev.target().value())
                                />
                                <button class="btn ghost" type="submit" disabled=move || busy.get()>
                                    {move || if busy.get() { "Opening the gate…" } else { "Enter without Discord" }}
                                </button>
                            </form>
                        }
                    });
                    view! {
                        {discord}
                        {dev}
                        {(!m.discord && !m.dev).then(|| view! { <p class="hint">"Sign-in is not available on this server yet."</p> })}
                    }
                        .into_any()
                }
            }}
        </section>
    }
}

/// Discord sends people back to the site here, carrying a one-time code in the address.
/// We trade it for a session and tidy the address bar.
#[component]
pub fn AuthCallback() -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    let notice = expect_context::<NoticeSignal>();
    let navigate = use_navigate();
    let query = use_query_map();
    let error = RwSignal::new(None::<String>);

    Effect::new(move |_| {
        let Some(code) = query.read().get("code") else {
            error.set(Some(
                "The sign-in link is missing its code. Please try again.".into(),
            ));
            return;
        };
        let navigate = navigate.clone();
        let next = query.read().get("next");
        spawn_local(async move {
            match api::post::<_, AuthResponse>("/auth/exchange", None, &ExchangeRequest { code })
                .await
            {
                Ok(auth) => {
                    let s = api::Session {
                        token: auth.token,
                        user: auth.user,
                    };
                    api::store_session(Some(&s));
                    let target = if next.as_deref() == Some("friends") {
                        "/friends"
                    } else {
                        landing_after_login(&s.token).await
                    };
                    accept_pending_invite(&s.token, notice).await;
                    session.set(Some(s));
                    // `replace` so the one-time code does not stay in the browser history.
                    navigate(
                        target,
                        NavigateOptions {
                            replace: true,
                            ..Default::default()
                        },
                    );
                }
                Err(e) => error.set(Some(e.text())),
            }
        });
    });

    view! {
        <section class="center-card">
            <div class="brand-mark huge">"友"</div>
            {move || match error.get() {
                None => view! {
                    <h2>"Opening the gate…"</h2>
                    <p class="muted">"Finishing your Discord sign-in."</p>
                }
                    .into_any(),
                Some(e) => view! {
                    <h2>"Sign-in did not finish"</h2>
                    <p class="error">{e}</p>
                    <A href="/login" attr:class="btn primary">"Try again"</A>
                }
                    .into_any(),
            }}
        </section>
    }
}

// ───────────────────────── Discover (ranked swipe deck) ─────────────────────────

#[derive(Clone)]
enum Dialog {
    Report(Profile),
    Block(Profile),
}

/// Plain-language reasons a card was suggested.
fn reasons(card: &DiscoverCard) -> Vec<String> {
    let mut r = Vec::new();
    if card.is_friend {
        r.push("Your friend".into());
    }
    match card.shared_servers.len() {
        0 => {}
        1 => r.push(format!("Both in {}", card.shared_servers[0])),
        n => r.push(format!("{n} shared servers")),
    }
    match card.shared_games.len() {
        0 => {}
        1 => r.push(format!("Both play {}", card.shared_games[0])),
        n => r.push(format!("{n} shared games")),
    }
    if card.same_region {
        r.push("Same region".into());
    }
    match card.rank_gap {
        Some(0) => r.push("Same rank".into()),
        Some(1) => r.push("Close rank".into()),
        _ => {}
    }
    r
}

#[component]
pub fn Discover() -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    view! {
        <section class="page">
            {move || match session.get() {
                None => view! { <SignInPrompt what="find teammates" /> }.into_any(),
                Some(_) => view! { <DiscoverDeck /> }.into_any(),
            }}
        </section>
    }
}

#[component]
fn DiscoverDeck() -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    let deck = RwSignal::new(Vec::<DiscoverCard>::new());
    let idx = RwSignal::new(0usize);
    let anim = RwSignal::new(None::<bool>); // Some(true) = sealing a pact, Some(false) = passing
    let matched = RwSignal::new(None::<MatchedWith>);
    let error = RwSignal::new(None::<String>);
    let need_profile = RwSignal::new(false);
    let loading = RwSignal::new(true);
    let dialog = RwSignal::new(None::<Dialog>);

    let load = move || {
        let Some(s) = session.get_untracked() else {
            return;
        };
        loading.set(true);
        error.set(None);
        spawn_local(async move {
            match api::get::<Vec<DiscoverCard>>("/discover", Some(&s.token)).await {
                Ok(v) => {
                    deck.set(v);
                    idx.set(0);
                    need_profile.set(false);
                }
                Err(ApiErr::Conflict(_)) => need_profile.set(true),
                Err(e) => error.set(Some(api::handle_err(session, e))),
            }
            loading.set(false);
        });
    };
    Effect::new(move |_| load());

    let current = move || deck.with(|d| d.get(idx.get()).cloned());
    let remaining = move || deck.with(|d| d.len()).saturating_sub(idx.get());

    let swipe = move |like: bool| {
        if anim.get_untracked().is_some()
            || matched.get_untracked().is_some()
            || dialog.get_untracked().is_some()
        {
            return;
        }
        let Some(card) = current() else { return };
        let Some(s) = session.get_untracked() else {
            return;
        };
        anim.set(Some(like));
        spawn_local(async move {
            let req = SwipeRequest {
                target_id: card.profile.user_id,
                liked: like,
            };
            match api::post::<_, SwipeResponse>("/swipe", Some(&s.token), &req).await {
                Ok(r) => {
                    if let Some(m) = r.matched {
                        matched.set(Some(m));
                    }
                }
                Err(e) => error.set(Some(api::handle_err(session, e))),
            }
        });
        set_timeout(
            move || {
                idx.update(|i| *i += 1);
                anim.set(None);
            },
            Duration::from_millis(280),
        );
    };

    view! {
        <div class="page-head">
            <h2>"Discover"</h2>
            <span class="muted">{move || format!("{} warriors remain", remaining())}</span>
        </div>

        {move || error.get().map(|e| view! { <p class="error">{e}</p> })}

        <div class="deck">
            {move || {
                if loading.get() {
                    return view! { <p class="muted">"Summoning warriors…"</p> }.into_any();
                }
                if need_profile.get() {
                    return view! {
                        <div class="empty">
                            <div class="emoji">"🗡️"</div>
                            <h3>"Forge your legend first"</h3>
                            <p class="muted">"Create your profile so others can find you too."</p>
                            <A href="/profile" attr:class="btn primary">"Create profile"</A>
                        </div>
                    }
                        .into_any();
                }
                match current() {
                    Some(card) => {
                        let hue = card.profile.hue;
                        let why = reasons(&card);
                        view! {
                            <article
                                class="pcard"
                                class:fly-right=move || anim.get() == Some(true)
                                class:fly-left=move || anim.get() == Some(false)
                                style=format!("--h:{hue}")
                            >
                                <span class="stamp like">"同盟"</span>
                                <span class="stamp nope">"去れ"</span>
                                <ProfileCard profile=card.profile reasons=why />
                            </article>
                        }
                            .into_any()
                    }
                    None => {
                        view! {
                            <div class="empty">
                                <div class="emoji">"🎮"</div>
                                <h3>"No warriors left on the field"</h3>
                                <p class="muted">"You've seen everyone for now. Check back as more people join."</p>
                                <button class="btn primary" on:click=move |_| load()>"Look again"</button>
                            </div>
                        }
                            .into_any()
                    }
                }
            }}
        </div>

        <div class="actions">
            <button class="round nope" title="Pass" on:click=move |_| swipe(false)>"去"</button>
            <button class="round like" title="Seal the pact" on:click=move |_| swipe(true)>"契"</button>
        </div>

        {move || {
            current()
                .map(|card| {
                    let p1 = card.profile.clone();
                    let p2 = card.profile;
                    view! {
                        <div class="card-links">
                            <button class="link" on:click=move |_| dialog.set(Some(Dialog::Report(p1.clone())))>"⚑ Report"</button>
                            <button class="link" on:click=move |_| dialog.set(Some(Dialog::Block(p2.clone())))>"⛔ Block"</button>
                        </div>
                    }
                })
        }}

        {move || {
            dialog
                .get()
                .map(|d| {
                    let target_id = match &d {
                        Dialog::Report(p) | Dialog::Block(p) => p.user_id,
                    };
                    let close = Callback::new(move |blocked: bool| {
                        if blocked {
                            deck.update(|v| v.retain(|c| c.profile.user_id != target_id));
                        }
                        dialog.set(None);
                    });
                    match d {
                        Dialog::Report(p) => view! { <ReportDialog target=p on_close=close /> }.into_any(),
                        Dialog::Block(p) => view! { <BlockDialog target=p on_close=close /> }.into_any(),
                    }
                })
        }}

        {move || {
            matched
                .get()
                .map(|m| {
                    let id = m.match_id;
                    let p = m.profile;
                    view! {
                        <div class="modal-back">
                            <div class="modal">
                                <p class="eyebrow">"契 · A pact is sealed"</p>
                                <h2>"You and " {p.display_name.clone()} " fight well together"</h2>
                                <div class="pair">
                                    <span class="spark">"✦"</span>
                                    <Avatar emoji=p.avatar.clone() hue=p.hue big=true image=p.avatar_url.clone() />
                                    <span class="spark">"✦"</span>
                                </div>
                                <div class="row">
                                    <A href=format!("/chat/{id}") attr:class="btn primary">"Send a scroll"</A>
                                    <button class="btn ghost" on:click=move |_| matched.set(None)>"Keep hunting"</button>
                                </div>
                            </div>
                        </div>
                    }
                })
        }}
    }
}

// ───────────────────────── Matches ─────────────────────────

#[component]
pub fn Matches() -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    let chat = expect_context::<ChatClient>();
    // Opening the page always shows fresh previews and unread counts.
    Effect::new(move |_| chat.refresh(session));

    view! {
        <section class="page">
            <div class="page-head"><h2>"Matches"</h2></div>
            {move || {
                let Some(me) = session.get().map(|s| s.user.id) else {
                    return view! { <SignInPrompt what="see your pacts" /> }.into_any();
                };
                if !chat.loaded.get() {
                    return view! { <p class="muted">"Gathering your allies…"</p> }.into_any();
                }
                let list = chat.matches.get();
                if list.is_empty() {
                    return view! {
                        <div class="empty">
                            <div class="emoji">"💤"</div>
                            <h3>"No pacts yet"</h3>
                            <p class="muted">"When you and another player both seal the pact, they appear here."</p>
                            <A href="/discover" attr:class="btn primary">"Go discover"</A>
                        </div>
                    }
                        .into_any();
                }
                view! {
                    <div class="list">
                        {list
                            .into_iter()
                            .map(|m| {
                                let id = m.match_id;
                                let p = m.profile;
                                let preview = match &m.last_message {
                                    Some(l) if l.sender_id == me => format!("You: {}", l.body),
                                    Some(l) => l.body.clone(),
                                    None => "Say hello and sharpen the pact.".to_string(),
                                };
                                let when = m.last_message.as_ref().map(|l| fmt_time(l.created_at_ms));
                                let unread = m.unread;
                                // Computed here: a bare `>` inside a view! attribute would end the tag.
                                let has_unread = unread > 0;
                                view! {
                                    <A href=format!("/chat/{id}") attr:class="row-item">
                                        <Avatar emoji=p.avatar.clone() hue=p.hue image=p.avatar_url.clone() />
                                        <div class="grow">
                                            <b>{p.display_name.clone()}</b>
                                            <p class="muted preview" class:strong=has_unread>{preview}</p>
                                        </div>
                                        <div class="side">
                                            <span class="muted">{when}</span>
                                            {(unread > 0).then(|| view! { <span class="badge">{unread}</span> })}
                                        </div>
                                    </A>
                                }
                            })
                            .collect_view()}
                    </div>
                }
                    .into_any()
            }}
        </section>
    }
}

// ───────────────────────── Chat ─────────────────────────

#[component]
pub fn Chat() -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    let params = use_params_map();
    view! {
        <section class="page chat">
            {move || {
                let id = params.read().get("id").and_then(|s| s.parse::<Uuid>().ok());
                match (session.get(), id) {
                    (None, _) => view! { <SignInPrompt what="chat with your matches" /> }.into_any(),
                    (Some(_), None) => view! { <ChatMissing /> }.into_any(),
                    (Some(s), Some(match_id)) => view! { <ChatRoom match_id=match_id me=s.user.id /> }.into_any(),
                }
            }}
        </section>
    }
}

#[component]
fn ChatMissing() -> impl IntoView {
    view! {
        <div class="empty">
            <h3>"You have no pact with this player"</h3>
            <A href="/matches" attr:class="btn primary">"Back to matches"</A>
        </div>
    }
}

#[derive(Clone, Copy, PartialEq)]
enum RoomDialog {
    Report,
    Block,
}

#[component]
fn ChatRoom(match_id: Uuid, me: Uuid) -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    let chat = expect_context::<ChatClient>();
    let navigate = use_navigate();

    let messages = RwSignal::new(Vec::<ChatMessage>::new());
    let other_read = RwSignal::new(0i64);
    let has_more = RwSignal::new(false);
    let loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    let typing = RwSignal::new(false);
    let draft = RwSignal::new(String::new());
    let dialog = RwSignal::new(None::<RoomDialog>);
    let list_ref = NodeRef::<leptos::html::Div>::new();

    // Plain counters/timestamps that must not trigger re-renders.
    let typing_gen = StoredValue::new(0u64);
    let idle_gen = StoredValue::new(0u64);
    let last_typing_sent = StoredValue::new(0f64);
    let last_marked = StoredValue::new(0i64);
    let newest_seen = StoredValue::new(0i64);

    let summary = move || {
        chat.matches
            .with(|m| m.iter().find(|x| x.match_id == match_id).cloned())
    };

    // ── history ──
    let load = move || {
        let Some(s) = session.get_untracked() else {
            return;
        };
        spawn_local(async move {
            let path = format!("/matches/{match_id}/messages");
            match api::get::<ChatHistory>(&path, Some(&s.token)).await {
                Ok(h) => {
                    other_read.set(h.other_last_read);
                    has_more.set(h.has_more);
                    messages.set(h.messages);
                    error.set(None);
                }
                Err(e) => error.set(Some(api::handle_err(session, e))),
            }
            loading.set(false);
        });
    };
    Effect::new(move |_| load());

    let load_older = move || {
        let Some(s) = session.get_untracked() else {
            return;
        };
        let Some(first) = messages.with_untracked(|v| v.first().map(|m| m.id)) else {
            return;
        };
        spawn_local(async move {
            let path = format!("/matches/{match_id}/messages?before={first}");
            match api::get::<ChatHistory>(&path, Some(&s.token)).await {
                Ok(h) => {
                    has_more.set(h.has_more);
                    messages.update(|v| {
                        let mut older = h.messages;
                        older.append(v);
                        *v = older;
                    });
                }
                Err(e) => error.set(Some(api::handle_err(session, e))),
            }
        });
    };

    // ── "seen": tell the server whenever I can see their newest message ──
    Effect::new(move |_| {
        let online = chat.status.get() == ChatStatus::Online;
        let newest_theirs =
            messages.with(|v| v.iter().rev().find(|m| m.sender_id != me).map(|m| m.id));
        if let (true, Some(id)) = (online, newest_theirs) {
            if id > last_marked.get_value()
                && chat.send(ClientMsg::Read {
                    match_id,
                    up_to: id,
                })
            {
                last_marked.set_value(id);
                chat.clear_unread(match_id);
            }
        }
    });

    // ── live events ──
    let sub = chat.subscribe(move |ev| match ev {
        ServerMsg::Ready { .. } => load(), // reconnected: catch up on anything missed
        ServerMsg::Message { message } if message.match_id == match_id => {
            messages.update(|v| {
                if !v.iter().any(|m| m.id == message.id) {
                    v.push(message.clone());
                }
            });
            if message.sender_id != me {
                typing.set(false);
            }
        }
        ServerMsg::Typing {
            match_id: mid,
            user_id,
            typing: is_typing,
        } if *mid == match_id && *user_id != me => {
            typing.set(*is_typing);
            if *is_typing {
                // Typing signals expire on their own in case the "stopped" signal is lost.
                typing_gen.update_value(|g| *g += 1);
                let g = typing_gen.get_value();
                set_timeout(
                    move || {
                        if typing_gen.get_value() == g {
                            typing.set(false);
                        }
                    },
                    TYPING_DISPLAY,
                );
            }
        }
        ServerMsg::Read {
            match_id: mid,
            user_id,
            up_to,
        } if *mid == match_id && *user_id != me => {
            other_read.update(|x| *x = (*x).max(*up_to));
        }
        ServerMsg::Error { message } => error.set(Some(message.clone())),
        _ => {}
    });
    on_cleanup(move || chat.unsubscribe(sub));

    // ── keep the newest message in view ──
    Effect::new(move |_| {
        let newest = messages.with(|v| v.last().map(|m| m.id)).unwrap_or(0);
        if newest != newest_seen.get_value() {
            newest_seen.set_value(newest);
            request_animation_frame(move || {
                if let Some(el) = list_ref.get_untracked() {
                    el.set_scroll_top(el.scroll_height());
                }
            });
        }
    });

    // ── composing ──
    let stop_typing = move || {
        if last_typing_sent.get_value() > 0.0 {
            last_typing_sent.set_value(0.0);
            chat.send(ClientMsg::Typing {
                match_id,
                typing: false,
            });
        }
    };
    let on_input = move |value: String| {
        draft.set(value.clone());
        error.set(None);
        if value.trim().is_empty() {
            stop_typing();
            return;
        }
        let now = now_ms();
        if now - last_typing_sent.get_value() > TYPING_RESEND_MS {
            last_typing_sent.set_value(now);
            chat.send(ClientMsg::Typing {
                match_id,
                typing: true,
            });
        }
        idle_gen.update_value(|g| *g += 1);
        let g = idle_gen.get_value();
        set_timeout(
            move || {
                if idle_gen.get_value() == g {
                    stop_typing();
                }
            },
            TYPING_IDLE,
        );
    };
    let send = move || {
        let text = draft.get_untracked();
        if text.trim().is_empty() {
            return;
        }
        if chat.send(ClientMsg::Send {
            match_id,
            body: text,
        }) {
            draft.set(String::new());
            last_typing_sent.set_value(0.0);
            idle_gen.update_value(|g| *g += 1);
        } else {
            error.set(Some("You are offline. Reconnecting…".into()));
        }
    };

    let nav_for_close = navigate.clone();

    view! {
        {move || {
            if chat.loaded.get() && summary().is_none() {
                return view! { <ChatMissing /> }.into_any();
            }
            let name = summary().map(|s| s.profile.display_name).unwrap_or_default();
            view! {
                <div class="chat-head">
                    <A href="/matches" attr:class="back">"←"</A>
                    {move || {
                        summary()
                            .map(|s| {
                                view! {
                                    <Avatar emoji=s.profile.avatar.clone() hue=s.profile.hue image=s.profile.avatar_url.clone() />
                                    <div class="grow">
                                        <b>{s.profile.display_name.clone()}</b>
                                        <p class="muted">{s.profile.games.iter().map(|g| g.name.clone()).collect::<Vec<_>>().join(", ")}</p>
                                    </div>
                                }
                            })
                    }}
                    <span class="conn" class:on=move || chat.status.get() == ChatStatus::Online>
                        {move || match chat.status.get() {
                            ChatStatus::Online => "● Live",
                            ChatStatus::Connecting => "◌ Connecting",
                            ChatStatus::Offline => "○ Offline",
                        }}
                    </span>
                    <div class="head-actions">
                        <button class="link" on:click=move |_| dialog.set(Some(RoomDialog::Report))>"⚑ Report"</button>
                        <button class="link" on:click=move |_| dialog.set(Some(RoomDialog::Block))>"⛔ Block"</button>
                    </div>
                </div>

                <div class="bubbles" node_ref=list_ref>
                    {move || {
                        has_more
                            .get()
                            .then(|| {
                                view! {
                                    <button class="btn small ghost older" on:click=move |_| load_older()>
                                        "Load earlier scrolls"
                                    </button>
                                }
                            })
                    }}
                    {move || {
                        if loading.get() {
                            return view! { <p class="muted">"Unrolling the scrolls…"</p> }.into_any();
                        }
                        if messages.with(|v| v.is_empty()) {
                            return view! {
                                <div class="empty">
                                    <div class="emoji">"📜"</div>
                                    <h3>"No scrolls yet"</h3>
                                    <p class="muted">"Send the first message and plan your next match."</p>
                                </div>
                            }
                                .into_any();
                        }
                        let last_mine = messages.with(|v| v.iter().rev().find(|m| m.sender_id == me).map(|m| m.id));
                        let read = other_read.get();
                        messages
                            .get()
                            .into_iter()
                            .map(|m| {
                                let mine = m.sender_id == me;
                                let status = (mine && Some(m.id) == last_mine)
                                    .then_some(if m.id <= read { " · Seen ✓✓" } else { " · Sent ✓" });
                                view! {
                                    <div class="msg" class:mine=mine>
                                        <div class="bubble" class:mine=mine>{m.body.clone()}</div>
                                        <div class="meta">{fmt_time(m.created_at_ms)} {status}</div>
                                    </div>
                                }
                            })
                            .collect_view()
                            .into_any()
                    }}
                    {move || {
                        typing
                            .get()
                            .then(|| {
                                view! {
                                    <div class="typing">
                                        <span></span><span></span><span></span>
                                        <em>{summary().map(|s| s.profile.display_name).unwrap_or_default()} " is writing…"</em>
                                    </div>
                                }
                            })
                    }}
                </div>

                <form
                    class="composer"
                    on:submit=move |ev| {
                        ev.prevent_default();
                        send();
                    }
                >
                    <input
                        type="text"
                        maxlength="2000"
                        autocomplete="off"
                        placeholder=move || {
                            if chat.status.get() == ChatStatus::Online { "Write a scroll…" } else { "Reconnecting…" }
                        }
                        prop:value=move || draft.get()
                        on:input:target=move |ev| on_input(ev.target().value())
                    />
                    <button class="btn primary" type="submit" disabled=move || chat.status.get() != ChatStatus::Online>
                        "Send"
                    </button>
                </form>
                {move || error.get().map(|e| view! { <p class="error">{e}</p> })}

                {
                    let navigate = nav_for_close.clone();
                    move || {
                        let navigate = navigate.clone();
                        let target = summary().map(|s| s.profile);
                        dialog
                            .get()
                            .zip(target)
                            .map(|(kind, p)| {
                                // Blocking ends the conversation, so leave the room afterwards.
                                let close = Callback::new(move |blocked: bool| {
                                    dialog.set(None);
                                    if blocked {
                                        navigate("/matches", Default::default());
                                    }
                                });
                                match kind {
                                    RoomDialog::Report => view! { <ReportDialog target=p on_close=close /> }.into_any(),
                                    RoomDialog::Block => view! { <BlockDialog target=p on_close=close /> }.into_any(),
                                }
                            })
                    }
                }
                <span class="sr-only">{name}</span>
            }
                .into_any()
        }}
    }
}

// ───────────────────────── Profile editor ─────────────────────────

/// Edit your profile.
#[component]
pub fn ProfileEdit() -> impl IntoView {
    view! { <ProfileEditor /> }
}

/// First stop for a brand-new account: the same form, introduced as a welcome.
#[component]
pub fn Welcome() -> impl IntoView {
    view! { <ProfileEditor welcome=true /> }
}

#[component]
fn ProfileEditor(#[prop(optional)] welcome: bool) -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    let navigate = use_navigate();
    let discord = RwSignal::new(None::<DiscordInfo>); // who Discord says you are, if you signed in with it
    let use_discord = RwSignal::new(false); // show the Discord picture instead of the emoji

    let name = RwSignal::new(String::new());
    let bio = RwSignal::new(String::new());
    let avatar = RwSignal::new(AVATARS[0].to_string());
    let hue = RwSignal::new(10u16);
    let rank = RwSignal::new("Unranked".to_string());
    let region = RwSignal::new(REGIONS[0].to_string());
    let style = RwSignal::new(PLAYSTYLES[0].to_string());
    let mic = RwSignal::new(true);
    let games = RwSignal::new(Vec::<Game>::new());
    let adult = RwSignal::new(false); // the 18+ checkbox
    let already_adult = RwSignal::new(false); // confirmed earlier on this account

    let loaded = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let saved = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);

    // Load the saved profile (if any) once we know who is signed in.
    Effect::new(move |_| {
        let Some(s) = session.get() else { return };
        loaded.set(false);
        spawn_local(async move {
            match api::get::<Me>("/me", Some(&s.token)).await {
                Ok(me) => {
                    already_adult.set(me.adult_confirmed);
                    discord.set(me.discord.clone());
                    match me.profile {
                        Some(p) => {
                            use_discord.set(p.avatar_url.is_some());
                            name.set(p.display_name);
                            bio.set(p.bio);
                            avatar.set(p.avatar);
                            hue.set(p.hue);
                            rank.set(p.rank);
                            region.set(p.region);
                            style.set(p.playstyle);
                            mic.set(p.mic);
                            games.set(p.games);
                        }
                        None => {
                            // First time here: start from what Discord gave us.
                            use_discord.set(me.discord.is_some());
                            name.set(
                                me.discord
                                    .as_ref()
                                    .map(|d| d.display_name.clone())
                                    .unwrap_or(me.user.username),
                            );
                        }
                    }
                    loaded.set(true);
                }
                Err(e) => error.set(Some(api::handle_err(session, e))),
            }
        });
    });

    let input = move || ProfileInput {
        display_name: name.get(),
        bio: bio.get(),
        avatar: avatar.get(),
        hue: hue.get(),
        games: games.with(|g| g.iter().map(|x| x.app_id).collect()),
        rank: rank.get(),
        region: region.get(),
        playstyle: style.get(),
        mic: mic.get(),
        adult_confirmed: adult.get() || already_adult.get(),
        use_discord_avatar: use_discord.get() && discord.with(|d| d.is_some()),
    };

    let save = Callback::new(move |_: ()| {
        if saving.get_untracked() {
            return;
        }
        let Some(s) = session.get_untracked() else {
            return;
        };
        let body = input();
        if let Err(msg) = body.validate() {
            error.set(Some(msg));
            return;
        }
        if !body.adult_confirmed {
            error.set(Some("Please confirm that you are 18 or older.".into()));
            return;
        }
        error.set(None);
        saving.set(true);
        let navigate = navigate.clone();
        spawn_local(async move {
            match api::put::<_, Profile>("/me/profile", Some(&s.token), &body).await {
                Ok(_) => {
                    already_adult.set(true);
                    if welcome {
                        // First profile done: off to find teammates.
                        navigate("/discover", Default::default());
                    }
                    saved.set(true);
                    set_timeout(move || saved.set(false), Duration::from_millis(2500));
                }
                Err(e) => error.set(Some(api::handle_err(session, e))),
            }
            saving.set(false);
        });
    });

    let select = |label: &'static str, sig: RwSignal<String>, options: &'static [&'static str]| {
        view! {
            <label class="field">
                <span>{label}</span>
                <select on:change:target=move |ev| sig.set(ev.target().value())>
                    {options
                        .iter()
                        .map(|o| {
                            let o = *o;
                            view! { <option value=o selected=move || sig.get() == o>{o}</option> }
                        })
                        .collect_view()}
                </select>
            </label>
        }
    };

    let preview = move || Profile {
        user_id: Uuid::nil(),
        display_name: if name.get().trim().is_empty() {
            "Your name".into()
        } else {
            name.get()
        },
        bio: bio.get(),
        avatar: avatar.get(),
        avatar_url: if use_discord.get() {
            discord.get().map(|d| d.avatar_url)
        } else {
            None
        },
        hue: hue.get(),
        games: games.get(),
        rank: rank.get(),
        region: region.get(),
        playstyle: style.get(),
        mic: mic.get(),
    };

    view! {
        <section class="page">
            <div class="page-head">
                <h2>{move || if welcome { format!("Welcome, {}", name.get()) } else { "Your profile".to_string() }}</h2>
            </div>
            {move || {
                if session.get().is_none() {
                    return view! { <SignInPrompt what="forge your profile" /> }.into_any();
                }
                if !loaded.get() && error.get().is_none() {
                    return view! { <p class="muted">"Unrolling your scroll…"</p> }.into_any();
                }
                view! {
                    {(welcome && discord.get().is_some()).then(|| view! {
                        <div class="welcome-note">
                            <p class="eyebrow">"From Discord"</p>
                            <p>"We brought in your name and picture. Discord does not share which games you play or who your friends are with apps, so pick your games from Steam below. To bring friends along, use your invite link on the Friends page. Everything here can be changed later."</p>
                        </div>
                    })}
                    <div class="editor">
                        <form
                            class="form"
                            on:submit=move |ev| {
                                ev.prevent_default();
                                save.run(());
                            }
                        >
                            <label class="field">
                                <span>"Display name"</span>
                                <input
                                    type="text"
                                    maxlength="32"
                                    prop:value=move || name.get()
                                    on:input:target=move |ev| name.set(ev.target().value())
                                />
                            </label>
                            <label class="field">
                                <span>{move || format!("Bio ({}/280)", bio.get().chars().count())}</span>
                                <textarea
                                    rows="3"
                                    maxlength="280"
                                    placeholder="What do you play, and how do you like to play it?"
                                    prop:value=move || bio.get()
                                    on:input:target=move |ev| bio.set(ev.target().value())
                                ></textarea>
                            </label>
                            {move || {
                                discord
                                    .get()
                                    .map(|d| {
                                        view! {
                                            <label class="check discord-avatar">
                                                <img class="da-img" src=d.avatar_url.clone() alt="" referrerpolicy="no-referrer" />
                                                <input
                                                    type="checkbox"
                                                    prop:checked=move || use_discord.get()
                                                    on:change:target=move |ev| use_discord.set(ev.target().checked())
                                                />
                                                <span>"Use my Discord avatar"</span>
                                            </label>
                                        }
                                    })
                            }}
                            <div class="field">
                                <span>"Emblem"</span>
                                <div class="chips">
                                    {AVATARS
                                        .iter()
                                        .map(|a| {
                                            let a = *a;
                                            view! {
                                                <button
                                                    type="button"
                                                    class="chip pick emoji-pick"
                                                    class:on=move || avatar.get() == a
                                                    on:click=move |_| avatar.set(a.to_string())
                                                >
                                                    {a}
                                                </button>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            </div>
                            <label class="field">
                                <span>"Aura"</span>
                                <input
                                    type="range"
                                    min="0"
                                    max="360"
                                    prop:value=move || hue.get().to_string()
                                    on:input:target=move |ev| hue.set(ev.target().value().parse().unwrap_or(0))
                                />
                            </label>
                            <div class="two">
                                {select("Rank", rank, &RANKS)}
                                {select("Playstyle", style, &PLAYSTYLES)}
                            </div>
                            {select("Region", region, &REGIONS)}
                            <div class="field">
                                <span>"Games you play (from Steam)"</span>
                                <GamePicker selected=games />
                            </div>
                            <label class="check">
                                <input
                                    type="checkbox"
                                    prop:checked=move || mic.get()
                                    on:change:target=move |ev| mic.set(ev.target().checked())
                                />
                                <span>"I use a mic"</span>
                            </label>
                            {move || {
                                if already_adult.get() {
                                    view! { <p class="muted">"✓ Age confirmed: 18 or older"</p> }.into_any()
                                } else {
                                    view! {
                                        <label class="check age">
                                            <input
                                                type="checkbox"
                                                prop:checked=move || adult.get()
                                                on:change:target=move |ev| adult.set(ev.target().checked())
                                            />
                                            <span>"I confirm that I am 18 years old or older"</span>
                                        </label>
                                    }
                                        .into_any()
                                }
                            }}
                            {move || error.get().map(|e| view! { <p class="error">{e}</p> })}
                            <div class="row">
                                <button class="btn primary" type="submit" disabled=move || saving.get()>
                                    {move || if saving.get() { "Saving…" } else if welcome { "Enter the arena" } else { "Save profile" }}
                                </button>
                                {move || saved.get().then(|| view! { <span class="saved">"✓ Saved"</span> })}
                            </div>
                        </form>

                        <div class="preview">
                            <p class="eyebrow">"How others see you"</p>
                            <article class="pcard static" style=move || format!("--h:{}", hue.get())>
                                {move || view! { <ProfileCard profile=preview() /> }}
                            </article>
                        </div>
                    </div>
                    <BlockedList />
                }
                    .into_any()
            }}
        </section>
    }
}

/// People you have blocked, with a way to undo it.
#[component]
fn BlockedList() -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    let chat = expect_context::<ChatClient>();
    let version = RwSignal::new(0u32);
    let error = RwSignal::new(None::<String>);

    let list = LocalResource::new(move || {
        version.get();
        let token = session.get().map(|s| s.token);
        async move {
            match token {
                Some(t) => api::get::<Vec<Profile>>("/blocks", Some(&t)).await,
                None => Ok(vec![]),
            }
        }
    });

    let unblock = move |id: Uuid| {
        let Some(s) = session.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match api::delete_unit(&format!("/blocks/{id}"), Some(&s.token)).await {
                Ok(()) => {
                    version.update(|v| *v += 1);
                    chat.refresh(session);
                }
                Err(e) => error.set(Some(api::handle_err(session, e))),
            }
        });
    };

    view! {
        <div class="blocked">
            <h3>"Blocked players"</h3>
            {move || error.get().map(|e| view! { <p class="error">{e}</p> })}
            {move || match list.get() {
                None => view! { <p class="muted">"Checking…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="error">{e.text()}</p> }.into_any(),
                Some(Ok(v)) if v.is_empty() => view! { <p class="muted">"You haven't blocked anyone."</p> }.into_any(),
                Some(Ok(v)) => {
                    view! {
                        <div class="list">
                            {v
                                .into_iter()
                                .map(|p| {
                                    let id = p.user_id;
                                    view! {
                                        <div class="row-item static">
                                            <Avatar emoji=p.avatar.clone() hue=p.hue image=p.avatar_url.clone() />
                                            <div class="grow"><b>{p.display_name.clone()}</b></div>
                                            <button class="btn small ghost" on:click=move |_| unblock(id)>"Unblock"</button>
                                        </div>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

// ───────────────────────── 404 ─────────────────────────

#[component]
pub fn NotFound() -> impl IntoView {
    view! {
        <section class="center-card">
            <div class="brand-mark huge">"迷"</div>
            <h2>"Lost in the lobby"</h2>
            <p class="muted">"That page doesn't exist."</p>
            <A href="/" attr:class="btn primary">"Back home"</A>
        </section>
    }
}

// ───────────────────────── Friends ─────────────────────────

/// Fetch something from the API whenever the session or `version` changes.
fn fetch_resource<T: serde::de::DeserializeOwned + 'static>(
    session: SessionSignal,
    version: RwSignal<u32>,
    path: &'static str,
) -> LocalResource<Result<T, ApiErr>> {
    LocalResource::new(move || {
        version.get();
        let token = session.get().map(|s| s.token);
        async move {
            match token {
                Some(t) => api::get::<T>(path, Some(&t)).await,
                None => Err(ApiErr::Unauthorized),
            }
        }
    })
}

#[component]
pub fn Friends() -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    view! {
        <section class="page">
            <div class="page-head"><h2>"Friends"</h2></div>
            {move || match session.get() {
                None => view! { <SignInPrompt what="bring your friends" /> }.into_any(),
                Some(_) => view! { <FriendsContent /> }.into_any(),
            }}
        </section>
    }
}

#[component]
fn FriendsContent() -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    let refresh = RwSignal::new(0u32);
    let me = fetch_resource::<Me>(session, refresh, "/me");
    let invite = fetch_resource::<InviteInfo>(session, refresh, "/me/invite");
    let friends = fetch_resource::<Vec<Friend>>(session, refresh, "/friends");
    let servers = fetch_resource::<ServersStatus>(session, refresh, "/me/servers");
    let people = fetch_resource::<Vec<ServerPerson>>(session, refresh, "/servers/people");
    let copied = RwSignal::new(None::<bool>);
    let error = RwSignal::new(None::<String>);

    let copy = move |url: String| {
        spawn_local(async move {
            let ok = api::copy_to_clipboard(&url).await;
            copied.set(Some(ok));
            set_timeout(move || copied.set(None), Duration::from_secs(4));
        });
    };

    let unfriend = move |id: Uuid| {
        let Some(s) = session.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match api::delete_unit(&format!("/friends/{id}"), Some(&s.token)).await {
                Ok(()) => refresh.update(|v| *v += 1),
                Err(e) => error.set(Some(api::handle_err(session, e))),
            }
        });
    };

    let stop_sharing = move |_| {
        let Some(s) = session.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match api::delete_unit("/me/servers", Some(&s.token)).await {
                Ok(()) => refresh.update(|v| *v += 1),
                Err(e) => error.set(Some(api::handle_err(session, e))),
            }
        });
    };

    view! {
        {move || error.get().map(|e| view! { <p class="error">{e}</p> })}

        <div class="panel">
            <h3>"Bring your friends"</h3>
            <p class="muted">
                "Anyone who joins through your link becomes your friend, and you show up near the top of each other's Discover deck."
            </p>
            {move || match invite.get() {
                None => view! { <p class="muted">"Making your link…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="error">{e.text()}</p> }.into_any(),
                Some(Ok(i)) => {
                    let url = i.url.clone();
                    view! {
                        <div class="copy-row">
                            <input
                                type="text"
                                readonly=true
                                prop:value=url.clone()
                                on:focus:target=move |ev| ev.target().select()
                            />
                            <button class="btn primary" on:click=move |_| copy(url.clone())>"Copy link"</button>
                        </div>
                        {move || {
                            copied
                                .get()
                                .map(|ok| {
                                    let class = if ok { "saved" } else { "error" };
                                    let text = if ok {
                                        "Link copied. Send it to a friend."
                                    } else {
                                        "Could not copy automatically. Select the link and press Ctrl+C."
                                    };
                                    view! { <p class=class>{text}</p> }
                                })
                        }}
                    }
                        .into_any()
                }
            }}
        </div>

        <div class="panel">
            <h3>"Your friends"</h3>
            {move || match friends.get() {
                None => view! { <p class="muted">"Gathering your friends…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="error">{e.text()}</p> }.into_any(),
                Some(Ok(v)) if v.is_empty() => {
                    view! { <p class="muted">"No friends here yet. Share your link above and they will appear."</p> }.into_any()
                }
                Some(Ok(v)) => {
                    view! {
                        <div class="list">
                            {v
                                .into_iter()
                                .map(|f| {
                                    let id = f.profile.user_id;
                                    let p = f.profile;
                                    let games = p.games.iter().map(|g| g.name.clone()).collect::<Vec<_>>().join(", ");
                                    view! {
                                        <div class="row-item static">
                                            <Avatar emoji=p.avatar.clone() hue=p.hue image=p.avatar_url.clone() />
                                            <div class="grow">
                                                <b>{p.display_name.clone()}</b>
                                                <p class="muted preview">{games}</p>
                                            </div>
                                            <button class="btn small ghost" on:click=move |_| unfriend(id)>"Unfriend"</button>
                                        </div>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                        .into_any()
                }
            }}
        </div>

        <div class="panel">
            <h3>"Players from your Discord servers"</h3>
            {move || match (me.get(), servers.get()) {
                (Some(Ok(m)), Some(Ok(st))) => {
                    if m.discord.is_none() {
                        return view! {
                            <p class="muted">"Sign in with Discord to find players from the servers you are in."</p>
                        }
                            .into_any();
                    }
                    if !st.connected {
                        return view! {
                            <p class="muted">
                                "Find Tomo players who are in the same Discord communities as you. This is optional and off until you turn it on."
                            </p>
                            <ul class="plain-list">
                                <li>"Tomo asks Discord for your server list, once, when you connect."</li>
                                <li>"Only small communities count (2,000 members or fewer)."</li>
                                <li>"Other people only ever see the names of servers you both are in."</li>
                                <li>"You can stop sharing at any time, which deletes what we stored."</li>
                            </ul>
                            <a class="btn discord big" href=format!("{}/auth/discord?servers=1", api::API_URL)>
                                "Connect my Discord servers"
                            </a>
                        }
                            .into_any();
                    }
                    let shared = st
                        .servers
                        .iter()
                        .map(|s| view! { <span class="chip">{s.clone()}</span> })
                        .collect_view();
                    let list = match people.get() {
                        None => view! { <p class="muted">"Looking for people…"</p> }.into_any(),
                        Some(Err(e)) => view! { <p class="error">{e.text()}</p> }.into_any(),
                        Some(Ok(v)) if v.is_empty() => {
                            view! {
                                <p class="muted">
                                    "No one else from your servers is on Tomo yet. Invite them with your link above."
                                </p>
                            }
                                .into_any()
                        }
                        Some(Ok(v)) => {
                            view! {
                                <div class="list">
                                    {v
                                        .into_iter()
                                        .map(|sp| {
                                            let p = sp.profile;
                                            let chips = sp
                                                .shared_servers
                                                .iter()
                                                .map(|s| view! { <span class="why-chip">{s.clone()}</span> })
                                                .collect_view();
                                            view! {
                                                <div class="row-item static">
                                                    <Avatar emoji=p.avatar.clone() hue=p.hue image=p.avatar_url.clone() />
                                                    <div class="grow">
                                                        <b>{p.display_name.clone()}</b>
                                                        <div class="chips">{chips}</div>
                                                    </div>
                                                </div>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                                .into_any()
                        }
                    };
                    view! {
                        <p class="muted">"You are sharing these small servers:"</p>
                        <div class="chips">{shared}</div>
                        <div class="row">
                            <button class="btn small ghost" on:click=stop_sharing>"Stop sharing"</button>
                        </div>
                        <h4 class="sub">"Also here"</h4>
                        {list}
                    }
                        .into_any()
                }
                (Some(Err(e)), _) | (_, Some(Err(e))) => view! { <p class="error">{e.text()}</p> }.into_any(),
                _ => view! { <p class="muted">"Checking your Discord…"</p> }.into_any(),
            }}
        </div>
    }
}

/// A friend's invite link: shows who invited you and, once you are signed in, makes you friends.
#[component]
pub fn Invite() -> impl IntoView {
    let session = expect_context::<SessionSignal>();
    let notice = expect_context::<NoticeSignal>();
    let navigate = use_navigate();
    let params = use_params_map();
    let code = move || params.read().get("code").unwrap_or_default();

    let preview = LocalResource::new(move || {
        let c = code();
        async move { api::get::<InvitePreview>(&format!("/invites/{c}"), None).await }
    });

    // Not signed in yet: remember the link so it is used right after signing in.
    Effect::new(move |_| {
        if session.get().is_none() && matches!(preview.get(), Some(Ok(_))) {
            api::store_pending_invite(&code());
        }
    });

    // Already signed in: use the link straight away.
    let used = StoredValue::new(false);
    Effect::new(move |_| {
        let Some(s) = session.get() else { return };
        if used.get_value() {
            return;
        }
        used.set_value(true);
        let c = code();
        let navigate = navigate.clone();
        spawn_local(async move {
            let req = AcceptInviteRequest { code: c };
            match api::post::<_, AcceptInviteResponse>("/invites/accept", Some(&s.token), &req)
                .await
            {
                Ok(r) if r.already_friends => notice.set(Some(format!(
                    "You and {} are already friends.",
                    r.friend_name
                ))),
                Ok(r) => notice.set(Some(format!("You and {} are now friends!", r.friend_name))),
                Err(e) => notice.set(Some(api::handle_err(session, e))),
            }
            navigate("/friends", Default::default());
        });
    });

    view! {
        <section class="center-card">
            {move || match preview.get() {
                None => view! { <p class="muted">"Opening your invite…"</p> }.into_any(),
                Some(Err(_)) => {
                    view! {
                        <div class="brand-mark huge">"迷"</div>
                        <h2>"This invite link is not valid"</h2>
                        <p class="muted">"Ask your friend to send a fresh one."</p>
                        <A href="/" attr:class="btn primary">"Back home"</A>
                    }
                        .into_any()
                }
                Some(Ok(p)) => {
                    let emoji = p.inviter_emoji.clone().unwrap_or_else(|| "🥷".to_string());
                    let name = p.inviter_name.clone();
                    view! {
                        <Avatar emoji=emoji hue=210 big=true image=p.inviter_avatar_url.clone() />
                        <h2>{name.clone()} " invited you to Tomo"</h2>
                        <p class="muted">
                            "Find teammates by rank, region and the games you share. Join and you will be friends with "
                            {name}
                            " straight away."
                        </p>
                        {move || {
                            if session.get().is_some() {
                                view! { <p class="muted">"Adding your friend…"</p> }.into_any()
                            } else {
                                view! { <A href="/login" attr:class="btn primary big">"Sign in to accept"</A> }.into_any()
                            }
                        }}
                    }
                        .into_any()
                }
            }}
        </section>
    }
}
