-- Discord identity details, refreshed on every Discord sign-in.
-- The avatar URL is always built by the server from Discord's own data (never taken from a
-- client), and the CHECK below makes sure only Discord's image host can ever be stored.
ALTER TABLE users
    ADD COLUMN discord_avatar_url  TEXT CHECK (discord_avatar_url IS NULL OR discord_avatar_url LIKE 'https://cdn.discordapp.com/%'),
    ADD COLUMN discord_global_name TEXT;

-- A profile can show the person's Discord avatar instead of an emoji.
-- `avatar_url` is a server-maintained copy so profile queries need no extra join.
ALTER TABLE profiles
    ADD COLUMN use_discord_avatar BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN avatar_url TEXT CHECK (avatar_url IS NULL OR avatar_url LIKE 'https://cdn.discordapp.com/%');
