# 友 Tomo

**Find your clan.** A "swipe to find teammates" app for gamers, built end to end in Rust:
a ranked discovery deck, mutual matches, and real-time chat, with a dark Japanese theme.

> *Tomo* (友) means "friend" in Japanese.

<!-- Add screenshots here: docs/screenshots/discover.png, chat.png -->

## What it does

- **Ranked discovery.** Your deck is ordered by shared games, region and rank closeness. Each card shows
  why it was suggested ("2 shared games · Same region · Same rank").
- **Mutual matches.** Swipe right on someone who swiped right on you and a match is created.
- **Steam game picker.** Search the whole Steam catalogue (or browse what is most played right now), with banners, and pick up to 12 games. Games are matched by Steam app id, and names always come from Steam.
- **Friends by invite link.** Everyone has a link. Whoever joins through it becomes your friend and is boosted in your Discover deck.
- **Players from your Discord servers (opt-in).** Tick one box when signing in and Tomo finds other players in the same small Discord communities. Servers over 2,000 members are ignored, only shared server names are ever shown, and you can disconnect and delete it any time.- **Real-time chat.** WebSocket messages stored in Postgres, with typing indicators, "Sent" / "Seen"
  marks, unread badges, history paging and automatic reconnect.
- **Safety built in.** Block (both directions, effective immediately), report with reasons, and a
  recorded 18+ confirmation.
- **Discord sign-in** (OAuth2) with state checking and one-time login codes, so session tokens never appear in URLs.
- **Three dark themes** (Oni, Yūrei, Ronin), responsive from small phones to 4K.

## How it's built

```
Browser (Leptos, WebAssembly)  ──REST + WebSocket──▶  API (Rust, Axum)  ──▶  Postgres (Neon)
        │                                                  │
   shared crate: the request/response types and validation, used by both sides
                                                           └──▶  Discord (OAuth2 sign-in)
```

| Part | Tech |
|---|---|
| Website | Rust, [Leptos](https://leptos.dev) (client-side), Trunk |
| API | Rust, [Axum](https://github.com/tokio-rs/axum), Tokio, SQLx |
| Database | PostgreSQL on [Neon](https://neon.tech) |
| Live chat | Axum WebSockets with an in-memory connection hub |
| Hosting (free) | Render (API), Cloudflare Pages (site), GitHub Actions (CI/build) |

The workspace has three crates in `crates/`:

- `shared` holds the types both sides use, plus profile validation. The compiler keeps the server
  and the browser in agreement.
- `api` is the server: auth, ranking, matching, chat, safety.
- `web` is the Leptos single-page app.

A few decisions worth knowing about:

- **One WebSocket per user.** The first frame authenticates, so tokens never go in the URL or logs.
  Sending a message is a single SQL statement that checks match membership and blocks and inserts,
  so a block takes effect instantly with no race.
- **Ranking is plain, tested Rust.** Candidates are pulled in random order and scored by a pure
  function with unit tests, so ties stay shuffled instead of always showing the same people.
- **"Unauthorized" only ever means "your token is invalid".** A database hiccup is reported
  differently so the browser never deletes a valid login.

## Run it locally

You need Rust (with the `wasm32-unknown-unknown` target), [Trunk](https://trunkrs.dev) 0.21, and a
free [Neon](https://neon.tech) database.

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked --version "^0.21"

cp .env.example .env.local        # then put your Neon connection string in it
cargo run -p tomo-api             # API on http://localhost:8080 (applies migrations)

cd crates/web && trunk serve      # website on http://localhost:3001
```

For local development, `DEV_AUTH=true` in `.env.local` adds a username-only login so you don't need
a Discord app. To use Discord sign-in, create an app at the
[Discord developer portal](https://discord.com/developers/applications), add the redirect
`http://localhost:8080/auth/discord/callback`, and put the client id and secret in `.env.local`.

### Tests

```bash
cargo test -p tomo-shared -p tomo-api     # ranking, auth, rate limiter, config, Discord client
cargo check -p tomo-web --target wasm32-unknown-unknown
```

The Discord tests run against a fake Discord started inside the test, so they need no network or keys.

## Deploying

Free hosting guide in [DEPLOY.md](DEPLOY.md).

## Roadmap

- [x] Ranked discovery, matches, real-time chat, safety, Discord sign-in
- [ ] Deploy and add a live demo link
- [ ] Installable PWA, then an Android app
- [x] Steam game picker, friend invite links, opt-in Discord server matching
- [ ] Availability / timezone matching and "looking for" tags
- [ ] Moderation dashboard for reports
- [ ] Clans and squad building

## Credits

The background artwork is placeholder material for local development. Replace it with art you
own or have licensed before publishing a public demo (see `crates/web/assets/art/`).
