-- Bitbucket Server / simulated endpoints: the connection's API base was
-- hardcoded to api.bitbucket.org. Store it per-connection; cloud keeps
-- the default.
ALTER TABLE bitbucket_connection
    ADD COLUMN IF NOT EXISTS base_url varchar(255)
    NOT NULL DEFAULT 'https://api.bitbucket.org';
