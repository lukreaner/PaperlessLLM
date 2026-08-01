-- Snapshot the historical dimensions needed to materialize an accurate usage
-- event when an in-flight provider call is interrupted. Existing reservations
-- remain NULL and are reconciled using the current profile as a compatibility
-- fallback.
ALTER TABLE budget_reservations ADD COLUMN provider_preset TEXT;
ALTER TABLE budget_reservations ADD COLUMN model TEXT;
