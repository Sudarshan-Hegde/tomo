-- Users may sign in with Discord (discord_id) or, in dev only, by username alone.
ALTER TABLE users ALTER COLUMN discord_id DROP NOT NULL;
CREATE UNIQUE INDEX users_username_lower_idx ON users (lower(username));

-- Login sessions. Only a SHA-256 hash of the bearer token is stored.
CREATE TABLE sessions (
    token_hash  TEXT PRIMARY KEY,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL
);
CREATE INDEX sessions_user_idx ON sessions (user_id);

CREATE TABLE profiles (
    user_id       UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    display_name  TEXT NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 32),
    bio           TEXT NOT NULL DEFAULT '' CHECK (char_length(bio) <= 280),
    avatar        TEXT NOT NULL,
    hue           SMALLINT NOT NULL CHECK (hue BETWEEN 0 AND 360),
    rank          TEXT NOT NULL,
    region        TEXT NOT NULL,
    playstyle     TEXT NOT NULL,
    mic           BOOLEAN NOT NULL DEFAULT true,
    games         TEXT[] NOT NULL DEFAULT '{}',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX profiles_games_idx ON profiles USING GIN (games);

-- One row per (swiper, target). Re-swiping overwrites the earlier decision.
CREATE TABLE swipes (
    swiper_id   UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_id   UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    liked       BOOLEAN NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (swiper_id, target_id),
    CHECK (swiper_id <> target_id)
);
CREATE INDEX swipes_target_liked_idx ON swipes (target_id) WHERE liked;

-- A match is an unordered pair, stored with user_a < user_b so it is unique.
CREATE TABLE matches (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_a      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_b      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (user_a < user_b),
    UNIQUE (user_a, user_b)
);
CREATE INDEX matches_user_b_idx ON matches (user_b);

-- Used by the WebSocket chat step.
CREATE TABLE messages (
    id          BIGSERIAL PRIMARY KEY,
    match_id    UUID NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    sender_id   UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body        TEXT NOT NULL CHECK (char_length(body) BETWEEN 1 AND 2000),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX messages_match_idx ON messages (match_id, id);
