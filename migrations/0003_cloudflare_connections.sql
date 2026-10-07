-- Phase 3 remainder: per-team Cloudflare connections + tunnels.
-- Port of devpush cloudflare_connection (app/models.py ~370). One row per
-- (team, account); api_token/tunnel_token are AES-256-GCM ciphertext.

CREATE TABLE cloudflare_connection (
    id                  VARCHAR(32) PRIMARY KEY,
    team_id             VARCHAR(32) NOT NULL REFERENCES team(id),
    account_id          VARCHAR(64) NOT NULL,
    account_name        VARCHAR(255) NOT NULL DEFAULT '',
    auth_method         TEXT NOT NULL DEFAULT 'api_token'
                        CHECK (auth_method IN ('oauth', 'api_token')),
    api_token           TEXT NOT NULL,          -- AES-GCM ciphertext
    oauth_refresh_token TEXT,
    oauth_expires_at    TIMESTAMPTZ,
    tunnel_id           VARCHAR(64),
    tunnel_name         VARCHAR(255),
    tunnel_token        TEXT,                   -- AES-GCM ciphertext
    tunnel_container_id VARCHAR(64),
    created_by_user_id  BIGINT REFERENCES "user"(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (team_id, account_id)
);
CREATE INDEX ix_cfconn_team ON cloudflare_connection (team_id);
