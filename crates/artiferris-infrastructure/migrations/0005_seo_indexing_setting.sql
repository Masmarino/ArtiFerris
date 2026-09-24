-- Whether search engines may index the public catalog. Read from the public organization's row; off until an admin opts in.
ALTER TABLE system_settings ADD COLUMN IF NOT EXISTS seo_indexing_enabled BOOLEAN NOT NULL DEFAULT false;
