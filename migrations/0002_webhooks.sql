-- Phase 4: outbound deployment webhooks.
-- Ports devpush team_webhook / project_webhook (app/models.py ~1240/1302).
-- `secret` holds AES-256-GCM ciphertext when set (HMAC signing key).

CREATE TABLE team_webhook (
    id            VARCHAR(32) PRIMARY KEY,
    team_id       VARCHAR(32) NOT NULL REFERENCES team(id),
    name          VARCHAR(100) NOT NULL,
    url           VARCHAR(2048) NOT NULL,
    secret        TEXT,
    events        JSONB NOT NULL DEFAULT '[]',
    project_ids   JSONB,                 -- NULL = all projects in the team
    status        TEXT NOT NULL DEFAULT 'active'
                  CHECK (status IN ('active', 'disabled')),
    created_by_user_id BIGINT REFERENCES "user"(id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_team_webhook_team ON team_webhook (team_id);

CREATE TABLE project_webhook (
    id          VARCHAR(32) PRIMARY KEY,
    project_id  VARCHAR(32) NOT NULL REFERENCES project(id),
    name        VARCHAR(100) NOT NULL,
    url         VARCHAR(2048) NOT NULL,
    secret      TEXT,
    events      JSONB NOT NULL DEFAULT '[]',
    status      TEXT NOT NULL DEFAULT 'active'
                CHECK (status IN ('active', 'disabled')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_project_webhook_project ON project_webhook (project_id);
