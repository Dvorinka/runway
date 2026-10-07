-- Phase 5: audit_log + notification — port of devpush app/models.py
-- AuditLog (~1417) and Notification (~1474).

CREATE TABLE audit_log (
    id            BIGSERIAL PRIMARY KEY,
    user_id       BIGINT REFERENCES "user"(id) ON DELETE SET NULL,
    team_id       VARCHAR(32) REFERENCES team(id) ON DELETE SET NULL,
    project_id    VARCHAR(32) REFERENCES project(id) ON DELETE SET NULL,
    action        VARCHAR(50) NOT NULL,
    resource_type VARCHAR(50),
    resource_id   VARCHAR(64),
    detail        TEXT,
    ip_address    VARCHAR(45),
    user_agent    VARCHAR(256),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_audit_user ON audit_log (user_id);
CREATE INDEX ix_audit_team ON audit_log (team_id);
CREATE INDEX ix_audit_project ON audit_log (project_id);
CREATE INDEX ix_audit_action ON audit_log (action);
CREATE INDEX ix_audit_created ON audit_log (created_at);

CREATE TABLE notification (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    team_id    VARCHAR(32) REFERENCES team(id) ON DELETE CASCADE,
    project_id VARCHAR(32) REFERENCES project(id) ON DELETE CASCADE,
    type       VARCHAR(50) NOT NULL,
    title      VARCHAR(200) NOT NULL,
    body       TEXT,
    link       VARCHAR(500),
    read       BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_notif_user ON notification (user_id);
CREATE INDEX ix_notif_read ON notification (read);
CREATE INDEX ix_notif_created ON notification (created_at);
