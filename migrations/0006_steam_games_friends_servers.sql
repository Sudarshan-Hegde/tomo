-- ───────── Games now come from Steam ─────────
-- `games` caches every Steam game our server has seen (from search or the popular list).
-- A profile may only reference games in this table, so names always come from Steam,
-- never from a client.
CREATE TABLE games (
    app_id      INTEGER PRIMARY KEY,
    name        TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 300),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE profiles ADD COLUMN game_ids INTEGER[] NOT NULL DEFAULT '{}';

-- Carry over what the old fixed list can be mapped to. Valorant, League of Legends, Fortnite,
-- Minecraft and Genshin are not on Steam, so those picks cannot be kept.
INSERT INTO games (app_id, name) VALUES
    (730, 'Counter-Strike 2'), (570, 'Dota 2'), (1172470, 'Apex Legends'),
    (2357570, 'Overwatch 2'), (252950, 'Rocket League')
ON CONFLICT DO NOTHING;

UPDATE profiles p SET game_ids = ARRAY(
    SELECT DISTINCT m.id
    FROM unnest(p.games) AS n
    JOIN (VALUES ('CS2', 730), ('Dota 2', 570), ('Apex Legends', 1172470),
                 ('Overwatch 2', 2357570), ('Rocket League', 252950)) AS m(name, id) ON m.name = n
);

DROP INDEX IF EXISTS profiles_games_idx;
ALTER TABLE profiles DROP COLUMN games;
CREATE INDEX profiles_game_ids_idx ON profiles USING GIN (game_ids);

-- ───────── Friends via invite links ─────────
ALTER TABLE users
    ADD COLUMN invite_code TEXT UNIQUE,
    ADD COLUMN invited_by UUID REFERENCES users(id) ON DELETE SET NULL;

-- An unordered pair stored with user_a < user_b, like matches.
CREATE TABLE friendships (
    user_a      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_b      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_a, user_b),
    CHECK (user_a < user_b)
);
CREATE INDEX friendships_b_idx ON friendships (user_b);

-- ───────── Discord servers (opt-in) ─────────
-- Rows exist only for people who chose "find players from my servers", and only for small
-- communities (see MAX_SERVER_MEMBERS in the API). Deleted when they disconnect.
CREATE TABLE user_guilds (
    user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    guild_id      TEXT NOT NULL,
    guild_name    TEXT NOT NULL CHECK (char_length(guild_name) BETWEEN 1 AND 200),
    member_count  INTEGER,
    PRIMARY KEY (user_id, guild_id)
);
CREATE INDEX user_guilds_guild_idx ON user_guilds (guild_id);

ALTER TABLE users ADD COLUMN servers_connected BOOLEAN NOT NULL DEFAULT false;

-- Remember, across the trip to Discord and back, whether this sign-in asked for the server list.
ALTER TABLE oauth_states ADD COLUMN with_servers BOOLEAN NOT NULL DEFAULT false;
