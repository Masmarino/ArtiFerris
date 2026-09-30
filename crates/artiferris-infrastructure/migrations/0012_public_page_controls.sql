-- Per organization (the public organization's row counts for the whole instance): whether its pages of the public catalog are served at all,
-- and whether search engines are kept away from them. Open and indexable unless an admin says otherwise, as before.
ALTER TABLE system_settings ADD COLUMN IF NOT EXISTS public_page_enabled BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE system_settings ADD COLUMN IF NOT EXISTS seo_indexing_blocked BOOLEAN NOT NULL DEFAULT false;
