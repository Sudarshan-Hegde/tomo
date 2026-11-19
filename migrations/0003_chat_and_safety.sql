-- 18+ confirmation: set once, when the user first creates a profile.
ALTER TABLE users ADD COLUMN adult_confirmed_at TIMESTAMPTZ;

-- Blocks hide both people from each other and stop messages in either direction.
CREATE TABLE blocks (
    blocker_id  UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blocked_id  UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (blocker_id, blocked_id),
    CHECK (blocker_id <> blocked_id)
);
CREATE INDEX blocks_blocked_idx ON blocks (blocked_id);

-- Reports are kept for review. One open report per (reporter, reported) pair.
CREATE TABLE reports (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    reporter_id  UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reported_id  UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason       TEXT NOT NULL,
    details      TEXT NOT NULL DEFAULT '' CHECK (char_length(details) <= 1000),
    status       TEXT NOT NULL DEFAULT 'open',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (reporter_id <> reported_id)
);
CREATE UNIQUE INDEX reports_open_unique ON reports (reporter_id, reported_id) WHERE status = 'open';

-- "Seen" marks: how far each person has read in each match.
CREATE TABLE match_reads (
    match_id      UUID NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    last_read_id  BIGINT NOT NULL DEFAULT 0,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (match_id, user_id)
);
