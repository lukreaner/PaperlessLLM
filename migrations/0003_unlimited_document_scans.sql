-- The original v1 schema silently limited every discovery pass to 50 rows.
-- Zero is the persisted representation of the explicit DocumentLimit::All
-- mode. Preserve any non-default limits that an administrator set manually.
UPDATE processing_profiles
SET max_documents_per_scan = 0
WHERE max_documents_per_scan = 50;
