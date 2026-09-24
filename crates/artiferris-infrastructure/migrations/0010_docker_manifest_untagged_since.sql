-- Idempotent throughout, like 0002 and 0003.

-- When the last tag moved away from a manifest, so the retention grace period runs from then and not from the first push.
-- NULL for a manifest that is tagged, or that was never tagged.
--
-- The manifests that are untagged when the column appears start their grace period now, so the first retention sweep after an
-- upgrade doesn't reclaim every old untagged manifest at once. Done only when the column is added, so a second run keeps the dates.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = current_schema() AND table_name = 'docker_manifests' AND column_name = 'untagged_since'
    ) THEN
        ALTER TABLE docker_manifests ADD COLUMN untagged_since TIMESTAMPTZ;
        UPDATE docker_manifests m SET untagged_since = now() WHERE NOT EXISTS (SELECT 1 FROM docker_tags t WHERE t.manifest_id = m.id);
    END IF;
END
$$;
