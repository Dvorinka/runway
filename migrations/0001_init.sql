-- Runway initial schema (Phase 1).
-- Ports the devpush reference model (app/models.py) 1:1 where Phase 1 needs
-- it: users/auth, teams, github installations, projects, deployments,
-- aliases, domains, tokens, and the Postgres job queue.
-- Secrets-bearing columns (env_vars, tokens) hold AES-256-GCM ciphertext.

CREATE TABLE "user" (
    id              BIGSERIAL PRIMARY KEY,
    email           VARCHAR(320) NOT NULL UNIQUE,
    username        VARCHAR(50) NOT NULL UNIQUE,
    name            VARCHAR(256),
    email_verified  BOOLEAN NOT NULL DEFAULT FALSE,
    status          TEXT NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active', 'deleted')),
    tokens_invalid_before TIMESTAMPTZ,
    default_team_id TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_user_email ON "user" (email);
CREATE INDEX ix_user_username ON "user" (username);

CREATE TABLE user_identity (
    id               BIGSERIAL PRIMARY KEY,
    user_id          BIGINT NOT NULL REFERENCES "user"(id),
    provider         TEXT NOT NULL CHECK (provider IN ('github', 'google')),
    provider_user_id VARCHAR(100),
    access_token     VARCHAR(4096),
    refresh_token    VARCHAR(4096),
    token_expires_at TIMESTAMPTZ,
    provider_metadata JSONB,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (provider, provider_user_id)
);
CREATE INDEX ix_user_identity_user ON user_identity (user_id);
CREATE INDEX ix_user_identity_puid ON user_identity (provider_user_id);

CREATE TABLE team (
    id                VARCHAR(32) PRIMARY KEY,
    name              VARCHAR(100) NOT NULL,
    slug              VARCHAR(40) UNIQUE,
    has_avatar        BOOLEAN NOT NULL DEFAULT FALSE,
    status            TEXT NOT NULL DEFAULT 'active'
                      CHECK (status IN ('active', 'deleted')),
    created_by_user_id BIGINT REFERENCES "user"(id) ON DELETE SET NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_team_name ON team (name);

ALTER TABLE "user"
    ADD CONSTRAINT fk_user_default_team
    FOREIGN KEY (default_team_id) REFERENCES team(id);

CREATE TABLE team_member (
    id         BIGSERIAL PRIMARY KEY,
    team_id    VARCHAR(32) NOT NULL REFERENCES team(id),
    user_id    BIGINT NOT NULL REFERENCES "user"(id),
    role       TEXT NOT NULL DEFAULT 'member'
               CHECK (role IN ('owner', 'admin', 'member')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (team_id, user_id)
);
CREATE INDEX ix_team_member_team ON team_member (team_id);
CREATE INDEX ix_team_member_user ON team_member (user_id);

CREATE TABLE github_installation (
    installation_id  BIGINT PRIMARY KEY,
    token            VARCHAR(4096),
    token_expires_at TIMESTAMPTZ,
    status           TEXT NOT NULL DEFAULT 'active'
                     CHECK (status IN ('active', 'deleted', 'suspended'))
);

CREATE TABLE project (
    id             VARCHAR(32) PRIMARY KEY,
    team_id        VARCHAR(32) NOT NULL REFERENCES team(id),
    name           VARCHAR(100) NOT NULL,
    slug           VARCHAR(64),
    description    VARCHAR(500) NOT NULL DEFAULT '',
    repo_provider  TEXT NOT NULL DEFAULT 'github'
                   CHECK (repo_provider IN ('github', 'gitea', 'gitlab', 'bitbucket', 'github_enterprise')),
    repo_id        BIGINT,
    repo_full_name VARCHAR(255) NOT NULL DEFAULT '',
    repo_base_url  VARCHAR(512) NOT NULL DEFAULT 'https://github.com',
    repo_branch    VARCHAR(255) NOT NULL DEFAULT '',
    repo_status    TEXT NOT NULL DEFAULT 'active'
                   CHECK (repo_status IN ('active', 'removed', 'deleted', 'transferred')),
    github_installation_id BIGINT REFERENCES github_installation(installation_id),
    gitea_connection_id    BIGINT,
    gitlab_connection_id   BIGINT,
    config         JSONB NOT NULL DEFAULT '{}',
    environments   JSONB NOT NULL DEFAULT '[]',
    env_vars       TEXT NOT NULL DEFAULT '',   -- AES-GCM ciphertext (JSON array)
    status         TEXT NOT NULL DEFAULT 'active'
                   CHECK (status IN ('active', 'deleting', 'deleted')),
    created_by_user_id BIGINT REFERENCES "user"(id) ON DELETE SET NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (team_id, name)
);
CREATE UNIQUE INDEX ix_project_team_name_lower ON project (team_id, lower(name));
CREATE INDEX ix_project_repo ON project (repo_id, repo_provider);
CREATE INDEX ix_project_slug ON project (slug);
CREATE INDEX ix_project_created ON project (created_at);

CREATE TABLE deployment (
    id              VARCHAR(32) PRIMARY KEY,
    project_id      VARCHAR(32) NOT NULL REFERENCES project(id),
    repo_provider   TEXT NOT NULL DEFAULT 'github'
                    CHECK (repo_provider IN ('github', 'gitea', 'gitlab', 'bitbucket', 'github_enterprise')),
    repo_id         BIGINT NOT NULL,
    repo_full_name  VARCHAR(255) NOT NULL,
    repo_base_url   VARCHAR(512) NOT NULL DEFAULT 'https://github.com',
    environment_id  VARCHAR(8) NOT NULL,
    branch          VARCHAR(255) NOT NULL,
    commit_sha      VARCHAR(40) NOT NULL,
    commit_meta     JSONB NOT NULL DEFAULT '{}',
    config          JSONB NOT NULL DEFAULT '{}',
    image           VARCHAR(512),
    env_vars        TEXT NOT NULL DEFAULT '',    -- AES-GCM ciphertext (JSON array)
    job_id          VARCHAR(36),
    error           JSONB,
    container_id    VARCHAR(64),
    container_status TEXT CHECK (container_status IN ('running', 'stopped', 'removed')),
    observed_status TEXT CHECK (observed_status IN
                    ('running', 'exited', 'dead', 'paused', 'not_found')),
    observed_exit_code INT,
    observed_at     TIMESTAMPTZ,
    observed_reason TEXT,
    observed_last_seen_at TIMESTAMPTZ,
    observed_missing_count INT NOT NULL DEFAULT 0,
    status          TEXT NOT NULL DEFAULT 'prepare'
                    CHECK (status IN ('prepare', 'deploy', 'finalize', 'fail', 'completed')),
    conclusion      TEXT CHECK (conclusion IN ('succeeded', 'failed', 'canceled', 'skipped')),
    trigger         TEXT NOT NULL DEFAULT 'user'
                    CHECK (trigger IN ('webhook', 'user', 'api')),
    created_by_user_id BIGINT REFERENCES "user"(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    concluded_at    TIMESTAMPTZ
);
CREATE INDEX ix_deployment_project ON deployment (project_id, created_at DESC);
CREATE INDEX ix_deployment_repo ON deployment (repo_id);
CREATE INDEX ix_deployment_branch ON deployment (branch);
CREATE INDEX ix_deployment_status ON deployment (status) WHERE conclusion IS NULL;

CREATE TABLE alias (
    id                      BIGSERIAL PRIMARY KEY,
    subdomain               VARCHAR(63) NOT NULL UNIQUE,
    deployment_id           VARCHAR(32) NOT NULL REFERENCES deployment(id),
    previous_deployment_id  VARCHAR(32) REFERENCES deployment(id),
    type                    TEXT NOT NULL
                            CHECK (type IN ('branch', 'environment', 'environment_id')),
    value                   VARCHAR(255),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_alias_deployment ON alias (deployment_id);
CREATE INDEX ix_alias_previous ON alias (previous_deployment_id);

CREATE TABLE domain (
    id                 BIGSERIAL PRIMARY KEY,
    project_id         VARCHAR(32) NOT NULL REFERENCES project(id),
    hostname           VARCHAR(255) NOT NULL,
    type               TEXT NOT NULL CHECK (type IN ('route', '301', '302', '307', '308')),
    environment_id     VARCHAR(8),
    status             TEXT NOT NULL DEFAULT 'pending'
                       CHECK (status IN ('pending', 'active', 'disabled', 'failed')),
    message            TEXT,
    last_checked_at    TIMESTAMPTZ,
    cloudflare_zone_id   VARCHAR(64),
    cloudflare_record_id VARCHAR(64),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_domain_project ON domain (project_id);
CREATE INDEX ix_domain_hostname ON domain (hostname);

CREATE TABLE deploy_token (
    id           VARCHAR(32) PRIMARY KEY,
    project_id   VARCHAR(32) NOT NULL REFERENCES project(id),
    name         VARCHAR(100) NOT NULL,
    token        VARCHAR(128) NOT NULL UNIQUE,   -- sha256 hex of raw token
    environment_id VARCHAR(8),                    -- NULL = all environments
    status       TEXT NOT NULL DEFAULT 'active'
                 CHECK (status IN ('active', 'revoked')),
    last_used_at TIMESTAMPTZ,
    created_by_user_id BIGINT REFERENCES "user"(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_deploy_token_project ON deploy_token (project_id);

CREATE TABLE api_key (
    id           VARCHAR(32) PRIMARY KEY,
    user_id      BIGINT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    name         VARCHAR(100) NOT NULL,
    token        VARCHAR(128) NOT NULL UNIQUE,   -- sha256 hex of raw token
    status       TEXT NOT NULL DEFAULT 'active'
                 CHECK (status IN ('active', 'revoked')),
    last_used_at TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_api_key_user ON api_key (user_id);

-- Postgres-backed job queue. Workers claim rows with
-- SELECT ... FOR UPDATE SKIP LOCKED. run_at enables deferred jobs
-- (e.g. delete_container after the grace period) and retry backoff.
CREATE TABLE job (
    id           BIGSERIAL PRIMARY KEY,
    kind         TEXT NOT NULL,
    payload      JSONB NOT NULL DEFAULT '{}',
    run_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    status       TEXT NOT NULL DEFAULT 'pending'
                 CHECK (status IN ('pending', 'running', 'done', 'failed')),
    attempts     INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 3,
    last_error   TEXT,
    locked_at    TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_job_pending ON job (run_at) WHERE status = 'pending';
