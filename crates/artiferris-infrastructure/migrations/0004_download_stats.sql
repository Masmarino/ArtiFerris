-- Daily download counts per package or image, for the "downloads this week" figures and the popularity sort.
-- Only counts, never who downloaded: no user and no address is recorded.
CREATE TABLE download_stats (
    day DATE NOT NULL,
    package_repository_id UUID NOT NULL REFERENCES package_repository_projections(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    downloads BIGINT NOT NULL,
    PRIMARY KEY (day, package_repository_id, kind, name)
);

CREATE INDEX download_stats_package_idx ON download_stats (package_repository_id, kind, name, day);
