-- The Overview and Activity pages poll recent job state while the worker is
-- active. These indexes keep that read path bounded as history grows.
CREATE INDEX IF NOT EXISTS idx_jobs_updated_at
    ON jobs(updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_jobs_status_updated_at
    ON jobs(status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_usage_events_job_id
    ON usage_events(job_id);
