-- Bitbucket project linkage — devpush never wires projects to a
-- bitbucket_connection; we do (it's the only way bitbucket repos deploy).
ALTER TABLE project
    ADD COLUMN bitbucket_connection_id BIGINT
        REFERENCES bitbucket_connection(id);
