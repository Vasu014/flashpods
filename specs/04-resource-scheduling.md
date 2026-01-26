# Resource Scheduling

## System Invariant

> **Flashpods never starts a job that would cause total allocated CPU or memory to exceed host capacity.**

No overcommit. No silent degradation. No surprise OOM kills.

## Admission Control

When creating a job, the API MUST:

1. Compute available resources:
```
available_cpus = host_total_cpus - sum(running_job_cpus)
available_memory = host_total_memory - sum(running_job_memory)
```

2. If requested resources exceed availability: reject with `429 Insufficient Resources`

## Rejection Response

```http
HTTP/1.1 429 Too Many Requests
Content-Type: application/json

{
  "error": "insufficient_resources",
  "message": "Not enough resources to start job",
  "requested": {
    "cpus": 4,
    "memory_gb": 8
  },
  "available": {
    "cpus": 2,
    "memory_gb": 6
  },
  "host_capacity": {
    "cpus": 8,
    "memory_gb": 16
  },
  "running_jobs": 3
}
```

## Queue Strategy: Reject (MVP)

**For MVP: No queue. Jobs are rejected when resources are unavailable.**

Rationale:
- MCP already handles retries via idempotency key
- Easier to reason about
- No background scheduler complexity
- Explicit failure is better than hidden waiting

**MCP handles rejection with backoff:**

```typescript
async function spawnWorkerWithBackoff(command: string, files: FileSpec): Promise<Job> {
  const clientJobId = crypto.randomUUID();

  for (let attempt = 0; attempt < 5; attempt++) {
    try {
      return await spawnWorker(command, files, { clientJobId });
    } catch (e) {
      if (e.status === 429) {
        const waitTime = Math.min(1000 * Math.pow(2, attempt), 30000);
        await sleep(waitTime);
        continue;
      }
      throw e;
    }
  }

  throw new Error('Resources unavailable after retries');
}
```

## No Preemption

**Flashpods does not preempt or evict running jobs to make room for new jobs.**

Once a job starts:
- It runs to completion, timeout, or cancellation
- No other job can take its resources
- No fairness-based eviction

This avoids:
- Killing long builds mid-execution
- Complex fairness policies
- State corruption from unexpected termination
- Unpredictable behavior

## Resource Caps by Job Type

| Type | Max CPUs | Max Memory | Rationale |
|------|----------|------------|-----------|
| Worker | 8 | 16 GB | Full host capacity for heavy builds |
| Sub-agent | 4 | 8 GB | Capped to prevent runaway delegation |

**Sub-agent caps are enforced even if host has more capacity.**

This prevents:
- Runaway recursive resource consumption
- Accidental host saturation from many sub-agents
- Single sub-agent starving other jobs

## Resource Tracking Query

```sql
SELECT
  SUM(cpus) as used_cpus,
  SUM(memory_gb) as used_memory_gb,
  COUNT(*) as running_jobs
FROM jobs
WHERE status IN ('starting', 'running');
```

## Admission Control Implementation

```rust
async fn can_admit_job(requested_cpus: i32, requested_memory_gb: i32) -> Result<(), AdmissionError> {
    let host = get_host_capacity();
    let used = get_current_usage().await;

    let available_cpus = host.cpus - used.cpus;
    let available_memory = host.memory_gb - used.memory_gb;

    if requested_cpus > available_cpus || requested_memory_gb > available_memory {
        return Err(AdmissionError::InsufficientResources {
            requested: Resources { cpus: requested_cpus, memory_gb: requested_memory_gb },
            available: Resources { cpus: available_cpus, memory_gb: available_memory },
            host_capacity: host,
            running_jobs: used.job_count,
        });
    }

    Ok(())
}

async fn create_job(req: CreateJobRequest) -> Result<Job, Error> {
    // 1. Validate job type resource caps
    let (max_cpus, max_memory) = match req.job_type {
        JobType::Worker => (8, 16),
        JobType::Agent => (4, 8),
    };

    let cpus = req.cpus.min(max_cpus);
    let memory_gb = req.memory_gb.min(max_memory);

    // 2. Check admission
    can_admit_job(cpus, memory_gb).await?;

    // 3. Create job (resources now "reserved")
    let job = insert_job(&req, cpus, memory_gb).await?;

    // 4. Start container
    start_container(&job).await?;

    Ok(job)
}
```

## Future: FIFO Queue (Post-MVP)

If needed later, add `queued` state:

```
pending → queued → starting → running → ...
```

Jobs enter `queued` when resources unavailable, scheduler starts them FIFO when resources free up.

## Related Specs

- [Jobs](./03-jobs.md) - Job lifecycle
- [API](./11-api.md) - HTTP endpoints
