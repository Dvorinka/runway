-- Container resource samples for succeeded/running deployments,
-- written by the monitor sweep. Powers the deployment-page sparklines.
CREATE TABLE IF NOT EXISTS deployment_metric (
    id BIGSERIAL PRIMARY KEY,
    deployment_id TEXT NOT NULL REFERENCES deployment(id) ON DELETE CASCADE,
    ts TIMESTAMPTZ NOT NULL DEFAULT now(),
    cpu_pct DOUBLE PRECISION NOT NULL DEFAULT 0,
    mem_used BIGINT NOT NULL DEFAULT 0,
    net_rx BIGINT NOT NULL DEFAULT 0,
    net_tx BIGINT NOT NULL DEFAULT 0,
    pids BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS deployment_metric_dep_ts ON deployment_metric (deployment_id, ts DESC);
