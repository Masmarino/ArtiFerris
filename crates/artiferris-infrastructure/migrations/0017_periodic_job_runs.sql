-- When each periodic job (retention sweep, metrics snapshot, ...) last started on any instance of a deployment, so that
-- the instances take turns instead of all running it at once.
CREATE TABLE IF NOT EXISTS periodic_job_runs (
    name TEXT PRIMARY KEY,
    last_started_at TIMESTAMPTZ NOT NULL
);
