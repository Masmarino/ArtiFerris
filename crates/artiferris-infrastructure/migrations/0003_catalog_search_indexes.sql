-- Substring and prefix search over Docker image names, same as npm_packages_name_trgm_idx does for npm.
CREATE INDEX IF NOT EXISTS docker_tags_image_name_trgm_idx ON docker_tags USING gin (image_name gin_trgm_ops);

-- Exact tag lookups (a search for "1.0" or "latest" finds the images carrying that tag).
CREATE INDEX IF NOT EXISTS docker_tags_lower_tag_idx ON docker_tags (lower(tag));

-- Description and keyword search across npm versions. The expression must match the query's exactly.
CREATE INDEX IF NOT EXISTS npm_package_versions_search_idx ON npm_package_versions USING gin (
    to_tsvector('simple', coalesce(manifest ->> 'description', '') || ' ' || coalesce(manifest ->> 'keywords', ''))
);
