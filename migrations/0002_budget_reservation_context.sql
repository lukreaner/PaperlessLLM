ALTER TABLE budget_reservations ADD COLUMN operation TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE budget_reservations ADD COLUMN api_family TEXT;
ALTER TABLE budget_reservations ADD COLUMN functions_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE budget_reservations ADD COLUMN input_tokens INTEGER NOT NULL DEFAULT 0;
