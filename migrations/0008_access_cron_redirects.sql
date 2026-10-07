-- Phase 5: sign-up allowlist + scheduled deployments + per-project
-- redirect rules — ports of devpush `allowlist` (~1219), `cron_job`
-- (~1553), `redirect_rule` (~1526).

CREATE TABLE allowlist (
    id         BIGSERIAL PRIMARY KEY,
    type       VARCHAR(20) NOT NULL CHECK (type IN ('email','domain','pattern')),
    value      VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_allowlist_type ON allowlist(type);
CREATE INDEX idx_allowlist_value ON allowlist(value);

CREATE TABLE cron_job (
    id             VARCHAR(32) PRIMARY KEY,
    project_id     VARCHAR(32) NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name           VARCHAR(100) NOT NULL,
    schedule       VARCHAR(100) NOT NULL,
    branch         VARCHAR(200) NOT NULL DEFAULT 'main',
    environment_id VARCHAR(50),
    enabled        BOOLEAN NOT NULL DEFAULT true,
    last_run_at    TIMESTAMPTZ,
    next_run_at    TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_cron_job_project ON cron_job(project_id);
CREATE INDEX idx_cron_job_due ON cron_job(next_run_at) WHERE enabled;

CREATE TABLE redirect_rule (
    id          VARCHAR(32) PRIMARY KEY,
    project_id  VARCHAR(32) NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    source_path VARCHAR(500) NOT NULL,
    target_url  VARCHAR(500) NOT NULL,
    status_code INTEGER NOT NULL DEFAULT 301,
    enabled     BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_redirect_rule_project ON redirect_rule(project_id);
