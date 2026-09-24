-- Idempotent throughout, like 0002 and 0003.

-- When a blob was linked to a repository, so the sweep can tell an upload still waiting for its manifest from an abandoned one.
ALTER TABLE docker_repository_blobs ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT now();
CREATE INDEX IF NOT EXISTS docker_repository_blobs_created_at_idx ON docker_repository_blobs (created_at);

-- Set once a complete request has taken over the session: a sealed session accepts no more chunks, so what was hashed is what gets stored.
ALTER TABLE docker_blob_uploads ADD COLUMN IF NOT EXISTS sealed_at TIMESTAMPTZ;

-- Versions that were published and then unpublished. npm never lets a version number come back with different bytes.
CREATE TABLE IF NOT EXISTS npm_unpublished_versions (
    package_repository_id UUID NOT NULL REFERENCES package_repository_projections(id) ON DELETE CASCADE,
    package_name TEXT NOT NULL,
    version TEXT NOT NULL,
    unpublished_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (package_repository_id, package_name, version)
);
