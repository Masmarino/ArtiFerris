-- Request counters of the anonymous-traffic limiter: how many requests one instance served for one client in one
-- one-minute window, so that several instances can share a budget. The client is a keyed hash, never an address. Unlogged:
-- a crash empties it, which only gives every client a fresh budget, and it is not worth the write-ahead log.
CREATE UNLOGGED TABLE IF NOT EXISTS rate_limit_counters (
    key_hash BYTEA NOT NULL,
    window_start BIGINT NOT NULL,
    instance_id UUID NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (key_hash, window_start, instance_id)
);
CREATE INDEX IF NOT EXISTS rate_limit_counters_window_idx ON rate_limit_counters (window_start);
