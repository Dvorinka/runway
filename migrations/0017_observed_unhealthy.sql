-- Allow 'unhealthy' observed state (HTTP health-check failures).

ALTER TABLE deployment DROP CONSTRAINT deployment_observed_status_check;
ALTER TABLE deployment ADD CONSTRAINT deployment_observed_status_check
    CHECK (observed_status IN
           ('running', 'exited', 'dead', 'paused', 'not_found', 'unhealthy'));
