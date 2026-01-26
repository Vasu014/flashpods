# Database Schema

## Overview

Flashpods uses SQLite with sqlx for type-safe queries.

**Location:** `/var/lib/flashpods/db/flashpods.db`

## Tables

### jobs

Main job tracking table.

```sql
CREATE TABLE jobs (
    id TEXT PRIMARY KEY,              -- UUID: job_abc123
    user_id TEXT NOT NULL DEFAULT 'default',
    job_type TEXT NOT NULL,           -- 'worker' or 'agent'
    status TEXT NOT NULL,             -- pending/starting/running/completed/failed/timed_out/cancelled/cleaning/cleaned

    -- Worker fields
    command TEXT,

    -- Agent fields
    task TEXT,
    context TEXT,
    git_branch TEXT,

    -- Common fields
    files_id TEXT,                    -- Reference to uploaded files
    image TEXT NOT NULL,
    cpus INTEGER NOT NULL DEFAULT 2,
    memory_gb INTEGER NOT NULL DEFAULT 4,
    timeout_minutes INTEGER NOT NULL DEFAULT 30,

    -- Runtime fields
    container_id TEXT,
    exit_code INTEGER,
    error TEXT,

    -- Timestamps
    created_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT
);

CREATE INDEX idx_jobs_user_id ON jobs(user_id);
CREATE INDEX idx_jobs_status ON jobs(status);
```

### idempotency_keys

Tracks idempotency keys for job creation.

```sql
CREATE TABLE idempotency_keys (
    client_job_id TEXT PRIMARY KEY,   -- Idempotency key from MCP
    job_id TEXT NOT NULL REFERENCES jobs(id),
    active INTEGER NOT NULL DEFAULT 1 -- 1 = active, 0 = job cleaned (key can be reused)
);

CREATE INDEX idx_idempotency_active ON idempotency_keys(client_job_id) WHERE active = 1;
```

### uploads

Tracks file upload lifecycle.

```sql
CREATE TABLE uploads (
    id TEXT PRIMARY KEY,              -- upload_abc123
    user_id TEXT NOT NULL DEFAULT 'default',
    state TEXT NOT NULL,              -- uploading/finalized/consumed/expired
    size_bytes INTEGER,
    file_count INTEGER,
    created_at TEXT NOT NULL,
    finalized_at TEXT,
    consumed_at TEXT,
    expires_at TEXT,
    job_id TEXT REFERENCES jobs(id)   -- Set when consumed
);

CREATE INDEX idx_uploads_state ON uploads(state);
CREATE INDEX idx_uploads_expires_at ON uploads(expires_at);
```

### artifacts

Tracks job artifacts.

```sql
CREATE TABLE artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES jobs(id),
    name TEXT NOT NULL,
    path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at TEXT NOT NULL,

    UNIQUE(job_id, name)  -- Enforce unique names per job
);

CREATE INDEX idx_artifacts_job_id ON artifacts(job_id);
```

## Common Queries

### Resource Usage

```sql
SELECT
  SUM(cpus) as used_cpus,
  SUM(memory_gb) as used_memory_gb,
  COUNT(*) as running_jobs
FROM jobs
WHERE status IN ('starting', 'running');
```

### Expired Uploads

```sql
SELECT id, state
FROM uploads
WHERE expires_at < ?
  AND state IN ('uploading', 'finalized');
```

### Active Jobs for User

```sql
SELECT * FROM jobs
WHERE user_id = ?
  AND status IN ('pending', 'starting', 'running')
ORDER BY created_at DESC;
```

## Idempotency Logic

```rust
async fn create_job_idempotent(req: CreateJobRequest) -> Result<(Job, bool), Error> {
    if let Some(client_job_id) = &req.client_job_id {
        // Check for existing active idempotency key
        if let Some(existing) = sqlx::query!(
            "SELECT job_id FROM idempotency_keys WHERE client_job_id = ? AND active = 1",
            client_job_id
        ).fetch_optional(&pool).await? {
            // Return existing job
            let job = get_job(&existing.job_id).await?;
            return Ok((job, false)); // created = false
        }
    }

    // Create new job
    let job = create_job_internal(&req).await?;

    // Insert idempotency key if provided
    if let Some(client_job_id) = &req.client_job_id {
        sqlx::query!(
            "INSERT INTO idempotency_keys (client_job_id, job_id, active) VALUES (?, ?, 1)",
            client_job_id, job.id
        ).execute(&pool).await?;
    }

    Ok((job, true)) // created = true
}

// Called when job transitions to 'cleaned'
async fn deactivate_idempotency_key(job_id: &str) {
    sqlx::query!(
        "UPDATE idempotency_keys SET active = 0 WHERE job_id = ?",
        job_id
    ).execute(&pool).await.ok();
}
```

## Related Specs

- [Jobs](./03-jobs.md) - Job lifecycle
- [Uploads](./05-uploads.md) - Upload lifecycle
- [Artifacts](./07-artifacts.md) - Artifact handling
