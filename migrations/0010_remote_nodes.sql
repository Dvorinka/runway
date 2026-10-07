-- Remote Docker nodes — port of devpush `remote_node` model +
-- `project.remote_node_id`. TLS material is stored AES-GCM encrypted
-- (devpush: fernet). devpush's admin UI only ever sets
-- name/host/docker_url; the TLS columns exist for schema parity.
CREATE TABLE remote_node (
    id              VARCHAR(32) PRIMARY KEY,
    name            VARCHAR(100) NOT NULL,
    host            VARCHAR(255) NOT NULL,
    docker_url      VARCHAR(255) NOT NULL,
    tls_ca          TEXT,
    tls_cert        TEXT,
    tls_key         TEXT,
    labels          JSONB NOT NULL DEFAULT '[]',
    status          TEXT NOT NULL DEFAULT 'online'
                    CHECK (status IN ('online', 'offline', 'disabled')),
    max_deployments INTEGER NOT NULL DEFAULT 10,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE project ADD COLUMN remote_node_id VARCHAR(32)
    REFERENCES remote_node(id) ON DELETE SET NULL;
