-- Discord login. Both tables hold short-lived, single-use secrets, stored hashed.

-- "state" ties the callback to the browser that started the login (stops forged callbacks).
CREATE TABLE oauth_states (
    state_hash  TEXT PRIMARY KEY,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL
);

-- After Discord approves, the browser is redirected with a one-time code that it trades for a
-- session. That keeps the real session token out of the address bar and browser history.
CREATE TABLE login_codes (
    code_hash   TEXT PRIMARY KEY,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at  TIMESTAMPTZ NOT NULL
);
