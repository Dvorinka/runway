-- Email verification + magic-link login tokens. Raw tokens are never
-- stored — sha256 only (same scheme as api_key/deploy_token).
CREATE TABLE IF NOT EXISTS email_token (
    id         VARCHAR(32) PRIMARY KEY,
    user_id    BIGINT REFERENCES "user"(id) ON DELETE CASCADE,
    email      TEXT NOT NULL,
    token      VARCHAR(64) NOT NULL UNIQUE,
    kind       VARCHAR(16) NOT NULL CHECK (kind IN ('verify', 'login')),
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS email_token_email_idx ON email_token(email);
