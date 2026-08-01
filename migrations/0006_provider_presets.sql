-- Keep the adapter family compatible with the original provider_kind CHECK
-- constraint while persisting the user-facing preset independently. Hosted
-- OpenAI-compatible presets use provider_kind='custom' and retain their own
-- identity here for the API, diagnostics, and usage statistics.
ALTER TABLE provider_profiles
ADD COLUMN provider_preset TEXT NOT NULL DEFAULT 'custom'
    CHECK(provider_preset IN (
        'openai',
        'claude',
        'lmstudio',
        'custom',
        'mistral',
        'kimi',
        'moonshot',
        'google'
    ));

UPDATE provider_profiles
SET provider_preset = provider_kind;
