-- Remote-node deployments: record which node served the deployment and
-- which host port it published, so the monitor/teardown talk to the
-- right daemon and Traefik can load-balance to node:port.
ALTER TABLE deployment
    ADD COLUMN remote_node_id VARCHAR(32) REFERENCES remote_node(id) ON DELETE SET NULL,
    ADD COLUMN remote_port INTEGER;
