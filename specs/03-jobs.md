# Job Lifecycle & Semantics

## Job Types

| Type | Purpose | /work Mode | Max CPUs | Max Memory |
|------|---------|------------|----------|------------|
| worker | Run commands (build, test, script) | ro | 8 | 16 GB |
| agent | Autonomous Claude instance | rw | 4 | 8 GB |

**Note:** The type value is `agent` (not "sub-agent"). Use `agent` consistently in code, labels, and API calls.

## Job States

```
┌─────────┐     ┌──────────┐     ┌─────────┐     ┌───────────┐     ┌──────────┐     ┌─────────┐
│ pending │ ──► │ starting │ ──► │ running │ ──► │ completed │ ──► │ cleaning │ ──► │ cleaned │
└─────────┘     └──────────┘     └─────────┘     │ failed    │     └──────────┘     └─────────┘
                                                 │ timed_out │
                                                 │ cancelled │
                                                 └───────────┘
```

| State | Description |
|-------|-------------|
| pending | Job created, waiting for upload finalization |
| starting | Container being created (image pull, namespace setup) |
| running | Command executing (container started successfully) |
| completed | Exit code 0 |
| failed | Exit code != 0, or container missing on reconciliation |
| timed_out | Exceeded timeout_minutes |
| cancelled | Killed via API |
| cleaning | Logs and artifacts being deleted |
| cleaned | Fully cleaned up, immutable |

## Exit Code Semantics

| Exit Code | Meaning |
|-----------|---------|
| 0 | Success |
| 1-125 | Command failure (application-defined) |
| 126 | Command not executable |
| 127 | Command not found |
| 128+N | Killed by signal N (e.g., 137 = 128+9 = SIGKILL) |
| 137 | OOM killed (SIGKILL from cgroup) or explicit SIGKILL |
| 143 | SIGTERM (graceful shutdown) |

**Special cases:**
- Timeout: Container killed with SIGTERM then SIGKILL → exit code 137, status `timed_out`
- Cancellation: Same as timeout → exit code 137, status `cancelled`
- OOM: Kernel OOM killer → exit code 137, status `failed`, error field contains "oom_killed"

## Core Principles

1. **Best-effort:** Jobs may fail due to resource limits, timeouts, or infrastructure issues. Flashpods does not guarantee completion.
2. **Non-resumable:** A failed/cancelled job cannot be resumed; create a new job instead
3. **Idempotent creation:** Same `client_job_id` returns existing job, not duplicate
4. **No automatic retries:** All retries are explicit and initiated by MCP

## Idempotency

**Request with idempotency key:**

```json
{
  "client_job_id": "550e8400-e29b-41d4-a716-446655440000",
  "type": "worker",
  "command": "cargo build --release",
  "files_id": "upload_abc123"
}
```

**Validation rules for `client_job_id`:**
- Must be valid UUID v4 format
- Max 36 characters
- Case-insensitive comparison

**Rule:**
- If job with same `client_job_id` exists and is NOT `cleaned`: return existing job (HTTP 200)
- If job is `cleaned`: create new job (old one is immutable history)

**Response indicates creation status:**

```json
{
  "job_id": "job_xyz789",
  "status": "running",
  "created": false,
  "message": "Existing job returned (idempotent)"
}
```

## Cancellation

**`kill_job(job_id)` flow:**

```
SIGTERM (to container PID 1) → grace period (10s) → SIGKILL (to all processes in cgroup) → cancelled
```

**Signal handling:**
- SIGTERM sent to container's PID 1 process
- Process should handle SIGTERM for graceful cleanup
- After grace period, SIGKILL sent to entire cgroup (all processes)
- Child processes cannot prevent shutdown after SIGKILL

**Invariants:**
- A cancelled job NEVER transitions to `completed`
- Retains logs/artifacts until TTL cleanup
- Is NOT resumable
- Cannot be un-cancelled

## Timeout

If runtime exceeds `timeout_minutes`:
1. Container is killed (same process as cancellation)
2. Job marked `timed_out`

**Response:**
```json
{
  "id": "job_xyz789",
  "status": "timed_out",
  "exit_code": 137,
  "timeout_minutes": 30,
  "actual_runtime_seconds": 1800,
  "error": "Job exceeded timeout limit"
}
```

## Cleanup

Cleanup **only occurs** when job is in terminal state: `completed | failed | timed_out | cancelled`

| State | Logs | Artifacts | Job Record |
|-------|------|-----------|------------|
| terminal states | Available | Available | Available |
| cleaning | Being deleted | Being deleted | Available |
| cleaned | Deleted | Deleted | Available (immutable) |

**After `cleaned`:**
- Job record remains in database (for history/debugging)
- Logs and artifacts are permanently deleted
- Idempotency key deactivated ONLY AFTER both logs and artifacts successfully deleted
- Same `client_job_id` can create a new job

## Crash Recovery

On API restart, reconcile jobs in `starting` or `running` state:

| Container Exists | DB State | Action |
|------------------|----------|--------|
| Yes, running | starting/running | Update to `running`, continue monitoring |
| Yes, exited | starting/running | Capture exit code, mark `completed` or `failed` |
| No | starting | Mark `failed`, error: "container_not_found_on_recovery" |
| No | running | Mark `failed`, error: "container_lost_on_recovery" |
| Yes | not in DB | Kill container (orphaned) |
| Podman error | any | Log warning, retry reconciliation, do not crash |

```rust
async fn reconcile_on_startup() {
    // 1. Get all flashpods containers (handle Podman errors gracefully)
    let containers = match podman_list_containers("flashpods-job=true").await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to list containers: {}. Retrying in 5s...", e);
            return; // Will retry on next reconciliation tick
        }
    };

    let container_job_ids: HashSet<_> = containers
        .iter()
        .filter_map(|c| c.labels.get("flashpods-job-id"))
        .collect();

    // 2. Get all non-terminal jobs from DB
    let active_jobs = db.query(
        "SELECT * FROM jobs WHERE status IN ('starting', 'running')"
    ).await;

    // 3. Reconcile
    for job in active_jobs {
        if let Some(container) = containers.iter().find(|c|
            c.labels.get("flashpods-job-id") == Some(&job.id)
        ) {
            if container.state == "running" {
                db.update_status(&job.id, "running").await;
            } else if container.state == "exited" {
                let exit_code = container.exit_code.unwrap_or(-1);
                let status = if exit_code == 0 { "completed" } else { "failed" };
                db.update_status(&job.id, status).await;
                db.set_exit_code(&job.id, exit_code).await;
            }
        } else {
            db.update_status(&job.id, "failed").await;
            db.set_error(&job.id, "container_lost_on_recovery").await;
        }
    }

    // 4. Kill orphaned containers
    for container in containers {
        let job_id = container.labels.get("flashpods-job-id");
        if let Some(id) = job_id {
            if !db.job_exists(id).await {
                warn!("Killing orphaned container for job {}", id);
                podman_kill(&container.id).await;
            }
        }
    }
}
```

## Default Resource Limits

| Resource | Worker | Agent |
|----------|--------|-------|
| Default CPUs | 2 | 2 |
| Max CPUs | 8 | 4 |
| Default Memory | 4 GB | 4 GB |
| Max Memory | 16 GB | 8 GB |
| Default Timeout | 30 min | 60 min |
| Max Timeout | 120 min | 120 min |

**Resource values are integers only.** Fractional CPUs or memory not supported. Requests are clamped to max, not rejected.

## Related Specs

- [Resource Scheduling](./04-resource-scheduling.md) - Admission control
- [Uploads](./05-uploads.md) - File upload lifecycle
- [Logs](./06-logs.md) - Log capture
- [Artifacts](./07-artifacts.md) - Output handling
- [Container Mounts](./15-container-mounts.md) - Mount paths and /work mode
- [Database Schema](./12-database.md) - Job tables
- [Error Codes](./17-error-codes.md) - All error responses
