mod api;
mod chat;
mod components;
mod pages;

use api::{load_session, Session};
use chat::ChatClient;
use components::{apply_theme, load_theme, BackgroundArt, Navbar};
use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use pages::{Chat, Discover, Landing, Login, Matches, NotFound, ProfileEdit};

#[component]
fn App() -> impl IntoView {
    // App-wide state, shared through context.
    let theme = RwSignal::new(load_theme());
    let session = RwSignal::new(load_session());
    provide_context(theme);
    provide_context::<RwSignal<Option<Session>>>(session);
    // One WebSocket + the shared matches/unread store, alive for as long as someone is signed in.
    provide_context(ChatClient::new(session));

    Effect::new(move |_| apply_theme(&theme.get()));

    view! {
        <BackgroundArt />
        <Router>
            <Navbar />
            <main class="main">
                <Routes fallback=NotFound>
                    <Route path=path!("/") view=Landing />
                    <Route path=path!("/login") view=Login />
                    <Route path=path!("/discover") view=Discover />
                    <Route path=path!("/matches") view=Matches />
                    <Route path=path!("/chat/:id") view=Chat />
                    <Route path=path!("/profile") view=ProfileEdit />
                </Routes>
            </main>
        </Router>
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
