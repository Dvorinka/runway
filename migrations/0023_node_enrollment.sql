-- Token-gated node enrollment: an install line on the node posts its
-- generated TLS material + inventory to /api/v1/nodes/checkin; the node
-- lands as 'waiting' until an admin approves it.

ALTER TABLE remote_node DROP CONSTRAINT IF EXISTS remote_node_status_check;
ALTER TABLE remote_node ADD CONSTRAINT remote_node_status_check
    CHECK (status IN ('online', 'offline', 'disabled', 'waiting'));

-- Inventory the node reported at checkin (docker version, arch, cpu/mem,
-- source IP). Displayed on the node row; not trusted for scheduling.
ALTER TABLE remote_node ADD COLUMN IF NOT EXISTS meta JSONB NOT NULL DEFAULT '{}';
-- Which token enrolled the node — revocation/audit trail.
ALTER TABLE remote_node ADD COLUMN IF NOT EXISTS token_id VARCHAR(32);

CREATE TABLE IF NOT EXISTS node_token (
    id          VARCHAR(32) PRIMARY KEY,
    name        VARCHAR(100) NOT NULL DEFAULT '',
    token_hash  VARCHAR(64) NOT NULL,          -- sha256 hex of the rnod_ token
    created_by  BIGINT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    expires_at  TIMESTAMPTZ NOT NULL,
    revoked_at  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS ix_node_token_expiry ON node_token (expires_at) WHERE revoked_at IS NULL;
ALTER TABLE remote_node DROP CONSTRAINT IF EXISTS remote_node_token_fk;
ALTER TABLE remote_node ADD CONSTRAINT remote_node_token_fk
    FOREIGN KEY (token_id) REFERENCES node_token(id) ON DELETE SET NULL;
