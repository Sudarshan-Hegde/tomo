use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;
use std::time::Duration;
use tomo_shared::{BlockRequest, Game, Health, Profile, ReportRequest, MAX_GAMES, REPORT_REASONS};

use crate::api::{self, Session};
use crate::chat::ChatClient;

pub const THEMES: [(&str, &str); 3] = [
    ("oni", "Oni 鬼"),
    ("yurei", "Yūrei 幽"),
    ("ronin", "Ronin 浪"),
];

pub fn load_theme() -> String {
    window()
        .local_storage()
        .ok()
        .flatten()
        .and_then(|s| s.get_item("tomo-theme").ok().flatten())
        .filter(|t| THEMES.iter().any(|(k, _)| k == t))
        .unwrap_or_else(|| "oni".to_string())
}

pub fn apply_theme(theme: &str) {
    if let Some(el) = document().document_element() {
        let _ = el.set_attribute("data-theme", theme);
    }
    if let Some(storage) = window().local_storage().ok().flatten() {
        let _ = storage.set_item("tomo-theme", theme);
    }
}

/// Gold ink warriors behind the whole app. The PNGs are the source drawings turned
/// into gold-on-transparent. Bump `?v=` when you replace one so browsers refetch it.
#[component]
pub fn BackgroundArt() -> impl IntoView {
    view! {
        <div class="bg-art" aria-hidden="true">
            <img class="art oni" src="/assets/art/oni.png?v=2" alt="" />
            <img class="art tengu" src="/assets/art/tengu.png?v=2" alt="" />
            <img class="art samurai" src="/assets/art/samurai.png?v=2" alt="" />
            <img class="art hannya" src="/assets/art/hannya.png?v=2" alt="" />
        </div>
    }
}

/// A round avatar: the person's picture if they have one (their Discord avatar), else their emoji.
#[component]
pub fn Avatar(
    emoji: String,
    hue: u16,
    #[prop(optional)] big: bool,
    #[prop(into, default = None)] image: Option<String>,
) -> impl IntoView {
    let content = match image {
        Some(url) => {
            view! { <img src=url alt="" referrerpolicy="no-referrer" loading="lazy" /> }.into_any()
        }
        None => emoji.into_any(),
    };
    view! {
        <div class="avatar" class:big=big style=format!("--h:{hue}")>
            {content}
        </div>
    }
}

#[component]
pub fn ApiStatus() -> impl IntoView {
    let health = LocalResource::new(|| async { api::get::<Health>("/health", None).await });
    view! {
        <span class="status-pill" title="Rust API + Neon database">
            {move || match health.get() {
                None => view! { <i class="dot wait"></i>"API…" }.into_any(),
                Some(Ok(h)) if h.database => view! { <i class="dot ok"></i>"API + DB online" }.into_any(),
                Some(Ok(_)) => view! { <i class="dot wait"></i>"API up, DB down" }.into_any(),
                Some(Err(_)) => view! { <i class="dot bad"></i>"API offline" }.into_any(),
            }}
        </span>
    }
}

#[component]
pub fn ThemeSwitcher() -> impl IntoView {
    let theme = expect_context::<RwSignal<String>>();
    view! {
        <div class="theme-switch" role="group" aria-label="Theme">
            {THEMES
                .iter()
                .map(|(key, label)| {
                    let key = key.to_string();
                    let k2 = key.clone();
                    view! {
                        <button
                            class="theme-btn"
                            class:active=move || theme.get() == k2
                            data-theme-preview=key.clone()
                            title=*label
                            on:click=move |_| theme.set(key.clone())
                        ></button>
                    }
                })
                .collect_view()}
        </div>
    }
}

#[component]
pub fn Navbar() -> impl IntoView {
    let session = expect_context::<RwSignal<Option<Session>>>();
    let chat = expect_context::<ChatClient>();
    let navigate = use_navigate();

    let sign_out = move |_| {
        if let Some(s) = session.get_untracked() {
            leptos::task::spawn_local(async move { api::logout(&s.token).await });
        }
        api::store_session(None);
        session.set(None);
        navigate("/", Default::default());
    };

    view! {
        <header class="nav">
            <A href="/" attr:class="brand">
                <span class="brand-mark">"友"</span>
                <span class="brand-name">"Tomo"</span>
            </A>
            <nav class="nav-links">
                <A href="/discover">"Discover"</A>
                <A href="/matches">
                    "Matches"
                    {move || {
                        let n = chat.unread.get();
                        (n > 0).then(|| view! { <span class="badge">{if n > 99 { "99+".to_string() } else { n.to_string() }}</span> })
                    }}
                </A>
                <A href="/friends">"Friends"</A>
                <A href="/profile">"Profile"</A>
            </nav>
            <div class="nav-right">
                <ThemeSwitcher />
                {move || match session.get() {
                    Some(s) => {
                        view! {
                            <span class="who">{s.user.username}</span>
                            <button class="btn small ghost" on:click=sign_out.clone()>"Leave"</button>
                        }
                            .into_any()
                    }
                    None => view! { <A href="/login" attr:class="btn small primary">"Sign in"</A> }.into_any(),
                }}
            </div>
        </header>
    }
}

#[component]
pub fn ProfileCard(profile: Profile, #[prop(optional)] reasons: Vec<String>) -> impl IntoView {
    // Each game shows its Steam banner as a small thumbnail. A missing banner just hides itself.
    let games = profile
        .games
        .iter()
        .take(8)
        .map(|g| {
            let banner = g.banner_url();
            view! {
                <span class="chip game">
                    <img class="gthumb" src=banner alt="" loading="lazy" on:error:target=move |ev| ev.target().set_hidden(true) />
                    {g.name.clone()}
                </span>
            }
        })
        .collect_view();
    let why = (!reasons.is_empty()).then(|| {
        view! {
            <div class="why">
                <span class="why-label">"Why you"</span>
                {reasons.iter().map(|r| view! { <span class="why-chip">{r.clone()}</span> }).collect_view()}
            </div>
        }
    });
    view! {
        <div class="pcard-body">
            <div class="pcard-top">
                <Avatar emoji=profile.avatar.clone() hue=profile.hue big=true image=profile.avatar_url.clone() />
                <div>
                    <h3>{profile.display_name.clone()}</h3>
                    <p class="muted">{profile.playstyle.clone()}</p>
                </div>
            </div>
            {why}
            <p class="bio">{profile.bio.clone()}</p>
            <div class="chips">{games}</div>
            <div class="facts">
                <span>"🏆 " {profile.rank.clone()}</span>
                <span>"🌏 " {profile.region.clone()}</span>
                <span>{if profile.mic { "🎙️ Mic on" } else { "🔇 No mic" }}</span>
            </div>
        </div>
    }
}

/// Shown on pages that need an account.
#[component]
pub fn SignInPrompt(#[prop(into)] what: String) -> impl IntoView {
    view! {
        <div class="empty">
            <div class="emoji">"🏯"</div>
            <h3>"The gate is closed"</h3>
            <p class="muted">{format!("Sign in to {what}.")}</p>
            <A href="/login" attr:class="btn primary">"Sign in"</A>
        </div>
    }
}

/// Confirm-and-block dialog. Calls `on_close(true)` if the player was blocked.
#[component]
pub fn BlockDialog(target: Profile, on_close: Callback<bool>) -> impl IntoView {
    let session = expect_context::<RwSignal<Option<Session>>>();
    let chat = expect_context::<ChatClient>();
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let id = target.user_id;

    let confirm = move |_| {
        let Some(s) = session.get_untracked() else {
            return;
        };
        busy.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match api::post_unit("/blocks", Some(&s.token), &BlockRequest { user_id: id }).await {
                Ok(()) => {
                    chat.refresh(session);
                    on_close.run(true);
                }
                Err(e) => {
                    error.set(Some(api::handle_err(session, e)));
                    busy.set(false);
                }
            }
        });
    };

    view! {
        <div class="modal-back" on:click=move |_| on_close.run(false)>
            <div class="modal left" on:click=|ev| ev.stop_propagation()>
                <p class="eyebrow">"Block"</p>
                <h2>"Block " {target.display_name.clone()} "?"</h2>
                <p class="muted">
                    "You will no longer see each other in Discover or Matches, and neither of you can send messages. "
                    "You can undo this any time from your Profile page."
                </p>
                {move || error.get().map(|e| view! { <p class="error">{e}</p> })}
                <div class="row end">
                    <button class="btn ghost" on:click=move |_| on_close.run(false)>"Cancel"</button>
                    <button class="btn danger" disabled=move || busy.get() on:click=confirm>
                        {move || if busy.get() { "Blocking…" } else { "Block" }}
                    </button>
                </div>
            </div>
        </div>
    }
}

/// Report dialog with an optional "also block". Calls `on_close(true)` if the player was blocked.
#[component]
pub fn ReportDialog(target: Profile, on_close: Callback<bool>) -> impl IntoView {
    let session = expect_context::<RwSignal<Option<Session>>>();
    let chat = expect_context::<ChatClient>();
    let reason = RwSignal::new(REPORT_REASONS[0].0.to_string());
    let details = RwSignal::new(String::new());
    let also_block = RwSignal::new(true);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let done = RwSignal::new(false);
    let id = target.user_id;

    let submit = move |_| {
        let Some(s) = session.get_untracked() else {
            return;
        };
        busy.set(true);
        error.set(None);
        let req = ReportRequest {
            user_id: id,
            reason: reason.get_untracked(),
            details: details.get_untracked(),
            also_block: also_block.get_untracked(),
        };
        leptos::task::spawn_local(async move {
            let blocked = req.also_block;
            match api::post_unit("/reports", Some(&s.token), &req).await {
                Ok(()) => {
                    chat.refresh(session);
                    done.set(true);
                    if blocked {
                        // Nothing more to do here; the caller moves on.
                        on_close.run(true);
                    }
                }
                Err(e) => {
                    error.set(Some(api::handle_err(session, e)));
                    if blocked {
                        chat.refresh(session); // the block is applied even if the report was a duplicate
                    }
                }
            }
            busy.set(false);
        });
    };

    view! {
        <div class="modal-back" on:click=move |_| on_close.run(false)>
            <div class="modal left" on:click=|ev| ev.stop_propagation()>
                <p class="eyebrow">"Report"</p>
                <h2>"Report " {target.display_name.clone()}</h2>
                {move || {
                    if done.get() {
                        return view! {
                            <p class="saved">"✓ Thank you. Your report was sent."</p>
                            <div class="row end">
                                <button class="btn primary" on:click=move |_| on_close.run(false)>"Close"</button>
                            </div>
                        }
                            .into_any();
                    }
                    view! {
                        <label class="field">
                            <span>"Reason"</span>
                            <select on:change:target=move |ev| reason.set(ev.target().value())>
                                {REPORT_REASONS
                                    .iter()
                                    .map(|(key, label)| view! { <option value=*key>{*label}</option> })
                                    .collect_view()}
                            </select>
                        </label>
                        <label class="field">
                            <span>{move || format!("Details, optional ({}/1000)", details.get().chars().count())}</span>
                            <textarea
                                rows="3"
                                maxlength="1000"
                                placeholder="What happened?"
                                prop:value=move || details.get()
                                on:input:target=move |ev| details.set(ev.target().value())
                            ></textarea>
                        </label>
                        <label class="check">
                            <input
                                type="checkbox"
                                prop:checked=move || also_block.get()
                                on:change:target=move |ev| also_block.set(ev.target().checked())
                            />
                            <span>"Also block this player"</span>
                        </label>
                        {move || error.get().map(|e| view! { <p class="error">{e}</p> })}
                        <div class="row end">
                            <button class="btn ghost" on:click=move |_| on_close.run(false)>"Cancel"</button>
                            <button class="btn danger" disabled=move || busy.get() on:click=submit>
                                {move || if busy.get() { "Sending…" } else { "Send report" }}
                            </button>
                        </div>
                    }
                        .into_any()
                }}
            </div>
        </div>
    }
}

/// Pick up to MAX_GAMES games from all of Steam: a "popular right now" shelf plus live search.
/// Every tile shows the game's Steam banner.
#[component]
pub fn GamePicker(selected: RwSignal<Vec<Game>>) -> impl IntoView {
    let session = expect_context::<RwSignal<Option<Session>>>();
    let query = RwSignal::new(String::new());
    let shelf = RwSignal::new(Vec::<Game>::new()); // what the grid currently shows
    let popular = RwSignal::new(Vec::<Game>::new());
    let heading = RwSignal::new("Popular on Steam right now".to_string());
    let loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    let limit_note = RwSignal::new(false);
    // Bumped on every keystroke, so a slow, older search can never overwrite a newer one.
    let search_gen = StoredValue::new(0u64);

    Effect::new(move |_| {
        let Some(s) = session.get_untracked() else {
            return;
        };
        leptos::task::spawn_local(async move {
            match api::get::<Vec<Game>>("/games/popular", Some(&s.token)).await {
                Ok(v) => {
                    popular.set(v.clone());
                    if query.get_untracked().trim().is_empty() {
                        shelf.set(v);
                    }
                }
                Err(e) => error.set(Some(api::handle_err(session, e))),
            }
            loading.set(false);
        });
    });

    let search = move |q: String| {
        let q = q.trim().to_string();
        search_gen.update_value(|g| *g += 1);
        let generation = search_gen.get_value();
        if q.chars().count() < 2 {
            shelf.set(popular.get_untracked());
            heading.set("Popular on Steam right now".into());
            loading.set(false);
            return;
        }
        let Some(s) = session.get_untracked() else {
            return;
        };
        loading.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            let encoded = js_sys::encode_uri_component(&q)
                .as_string()
                .unwrap_or_default();
            let result =
                api::get::<Vec<Game>>(&format!("/games/search?q={encoded}"), Some(&s.token)).await;
            if search_gen.get_value() != generation {
                return; // the person kept typing; a newer search owns the grid now
            }
            match result {
                Ok(v) => {
                    heading.set(format!("Results for \"{q}\""));
                    shelf.set(v);
                }
                Err(e) => error.set(Some(api::handle_err(session, e))),
            }
            loading.set(false);
        });
    };

    let on_input = move |value: String| {
        query.set(value.clone());
        search_gen.update_value(|g| *g += 1);
        let generation = search_gen.get_value();
        // Wait for a pause in typing before asking Steam.
        set_timeout(
            move || {
                if search_gen.get_value() == generation {
                    search(value.clone());
                }
            },
            Duration::from_millis(350),
        );
    };

    let toggle = move |game: Game| {
        limit_note.set(false);
        selected.update(|v| {
            if let Some(i) = v.iter().position(|g| g.app_id == game.app_id) {
                v.remove(i);
            } else if v.len() < MAX_GAMES {
                v.push(game);
            } else {
                limit_note.set(true);
            }
        });
    };

    view! {
        <div class="picker">
            <div class="picked">
                {move || {
                    let list = selected.get();
                    if list.is_empty() {
                        return view! { <p class="muted">"Nothing picked yet. Search below, or tap a game."</p> }.into_any();
                    }
                    list.into_iter()
                        .map(|g| {
                            let banner = g.banner_url();
                            let name = g.name.clone();
                            view! {
                                <button type="button" class="picked-game" title="Remove" on:click=move |_| toggle(g.clone())>
                                    <img src=banner alt="" loading="lazy" on:error:target=move |ev| ev.target().set_hidden(true) />
                                    <span>{name}</span>
                                    <b>"✕"</b>
                                </button>
                            }
                        })
                        .collect_view()
                        .into_any()
                }}
            </div>
            <p class="muted count">{move || format!("{} of {} picked", selected.with(|v| v.len()), MAX_GAMES)}</p>
            {move || limit_note.get().then(|| view! { <p class="error">{format!("You can pick up to {MAX_GAMES} games. Remove one to add another.")}</p> })}

            <input
                type="search"
                class="picker-search"
                placeholder="Search all of Steam…"
                maxlength="60"
                autocomplete="off"
                prop:value=move || query.get()
                on:input:target=move |ev| on_input(ev.target().value())
            />
            <p class="muted">{move || heading.get()}</p>
            {move || error.get().map(|e| view! { <p class="error">{e}</p> })}

            <div class="game-grid" class:busy=move || loading.get()>
                {move || {
                    let list = shelf.get();
                    if list.is_empty() && !loading.get() {
                        return view! { <p class="muted">"No games found. Try another name."</p> }.into_any();
                    }
                    list.into_iter()
                        .map(|g| {
                            let id = g.app_id;
                            let banner = g.banner_url();
                            let name = g.name.clone();
                            view! {
                                <button
                                    type="button"
                                    class="game-tile"
                                    class:on=move || selected.with(|v| v.iter().any(|x| x.app_id == id))
                                    on:click=move |_| toggle(g.clone())
                                >
                                    <span class="g-art">
                                        <img src=banner alt="" loading="lazy" on:error:target=move |ev| ev.target().set_hidden(true) />
                                    </span>
                                    <span class="g-name">{name}</span>
                                    <span class="g-check">"✓"</span>
                                </button>
                            }
                        })
                        .collect_view()
                        .into_any()
                }}
            </div>
        </div>
    }
}

/// A short message under the top menu, which hides itself after a few seconds.
#[component]
pub fn NoticeBar() -> impl IntoView {
    let notice = expect_context::<api::NoticeSignal>();
    let generation = StoredValue::new(0u64);
    Effect::new(move |_| {
        if notice.get().is_some() {
            generation.update_value(|g| *g += 1);
            let g = generation.get_value();
            set_timeout(
                move || {
                    if generation.get_value() == g {
                        notice.set(None);
                    }
                },
                Duration::from_secs(8),
            );
        }
    });
    view! {
        {move || {
            notice
                .get()
                .map(|m| {
                    view! {
                        <div class="notice" role="status">
                            <span>{m}</span>
                            <button class="link" on:click=move |_| notice.set(None)>"✕"</button>
                        </div>
                    }
                })
        }}
    }
}
