-- Phase 5: storage provisioning — port of devpush storage + storage_project
-- (app/models.py ~830). config holds engine/host/port/credentials (password
-- is sensitive but devpush stores it plaintext in config JSONB — we encrypt
-- it instead at write time; see worker/storage.rs).

CREATE TABLE storage (
    id                 VARCHAR(32) PRIMARY KEY,
    name               VARCHAR(100) NOT NULL,
    type               TEXT NOT NULL
                       CHECK (type IN ('database', 'volume', 'kv', 'queue')),
    status             TEXT NOT NULL DEFAULT 'pending'
                       CHECK (status IN ('pending','active','resetting','error','deleted')),
    config             JSONB NOT NULL DEFAULT '{}',
    error              JSONB,
    team_id            VARCHAR(32) NOT NULL REFERENCES team(id),
    created_by_user_id BIGINT REFERENCES "user"(id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (team_id, name)
);
CREATE UNIQUE INDEX ix_storage_team_name_lower ON storage (team_id, lower(name));

CREATE TABLE storage_project (
    id              VARCHAR(32) PRIMARY KEY,
    storage_id      VARCHAR(32) NOT NULL REFERENCES storage(id),
    project_id      VARCHAR(32) NOT NULL REFERENCES project(id),
    -- NULL/[] = all environments; otherwise restrict to these env ids.
    environment_ids JSONB,
    secrets         JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (storage_id, project_id)
);
CREATE INDEX ix_storage_project_project ON storage_project (project_id);
