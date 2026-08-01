-- Keep the provider selected in the dashboard in a tiny dedicated setting so
-- the frequently-polled status endpoint never has to parse the full settings
-- document (which may contain a long custom prompt).
INSERT INTO app_settings(key, value_json, updated_at)
SELECT
    'active_provider_id',
    json_quote(json_extract(value_json, '$.active_provider_id')),
    updated_at
FROM app_settings
WHERE key = 'dashboard_settings'
  AND CASE
      WHEN json_valid(value_json)
      THEN json_type(value_json, '$.active_provider_id') = 'text'
      ELSE 0
  END
ON CONFLICT(key) DO NOTHING;
