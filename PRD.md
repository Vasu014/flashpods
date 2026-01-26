# Flashpods: Ephemeral Instances for AI Coding Agents

## MVP Specification Document

**Project Codename:** Flashpods  
**Author:** Vasu Bhardwaj  
**Version:** 2.4  
**Date:** January 2026  
**Status:** Draft  
**Repository:** github.com/yourusername/flashpods

---

## Changelog

| Version | Date | Changes |
|---------|------|---------|
| 2.4 | Jan 2026 | SPIRE attestation fix, API bearer token, firewall policy, trust boundary, Podman invariants, validation tests, File Upload Lifecycle, Logs spec, Artifacts spec, Job Semantics (idempotency, cancellation, timeout, cleanup, crash recovery), Resource Scheduling (admission control, no overcommit, no preemption, job type caps) |
| 2.3 | Jan 2026 | Ephemeral model, rsync daemon, token exchange service, sub-agents can't spawn |
| 2.2 | Jan 2026 | Rebrand to Flashpods, workspace sync, artifact handling, SPIRE commitment |
| 2.1 | Jan 2026 | Cost analysis, testing strategy, risk register, eBPF observability plan |
| 2.0 | Jan 2026 | Major reframe: Agent-callable compute backend |
| 1.0 | Jan 2026 | Initial spec |

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [The Core Idea](#the-core-idea)
3. [Architecture Overview](#architecture-overview)
4. [Trust Boundary](#trust-boundary)
5. [Key Design Decisions](#key-design-decisions)
6. [Usage Patterns](#usage-patterns)
7. [Job Lifecycle](#job-lifecycle)
8. [Job Semantics](#job-semantics)
9. [Resource Scheduling](#resource-scheduling)
10. [Component Specifications](#component-specifications)
11. [File Upload (rsync Daemon)](#file-upload-rsync-daemon)
12. [File Upload Lifecycle](#file-upload-lifecycle)
13. [Logs](#logs)
14. [Artifacts](#artifacts)
15. [Identity & Secrets (SPIRE)](#identity--secrets-spire)
16. [Token Exchange Service](#token-exchange-service)
17. [MCP Tools](#mcp-tools)
18. [API Specification](#api-specification)
19. [Security Model](#security-model)
20. [NixOS Configuration](#nixos-configuration)
21. [Validation Tests](#validation-tests)
22. [Implementation Phases](#implementation-phases)
23. [Future Features](#future-features)
24. [Quick Reference](#quick-reference)

---

## Executive Summary

Flashpods is a **compute backend** that gives AI coding agents the ability to:

1. **Spawn workers** for long-running tasks (builds, tests, scripts)
2. **Spawn sub-agents** to work on tasks in parallel
3. **Execute code** in isolated, ephemeral containers
4. **Offload resource-heavy work** without blocking the main agent

### The One-Liner

> Your AI agent shouldn't wait 10 minutes for `cargo build`. It should spawn a worker, continue coding, and check back when it's done.

### Core Model

**Ephemeral containers.** Like E2B:
- Files go IN at job start (uploaded for that job)
- Files come OUT via /artifacts (explicitly retrieved)
- Container dies, everything gone

**No persistent workspaces.** No shared state between jobs.

---

## The Core Idea

### Before Flashpods

```
Agent: "Let me build this Rust project..."
     > cargo build --release
     [=================>          ] 45%
     
     *waiting 10 minutes*
     
Agent: "Ok NOW I can continue..."
```

### After Flashpods

```
Agent: "Let me build this Rust project..."
     > spawn_worker("cargo build --release", files: "/projects/myapp")
     ← job_id: "job_abc123"
     
     "Build running in background. Meanwhile, let me write the docs..."
     
     *productive work continues*
     
     > get_job_status("job_abc123")
     ← completed, exit_code: 0
     
     "Build done! Binary ready in artifacts."
```

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              YOUR LAPTOP                                     │
│                                                                              │
│  ┌─────────────────────────────────────────────────────────────────────────┐│
│  │                    AI Agent (Claude Code)                                ││
│  │                                                                          ││
│  │  Tools: spawn_worker, spawn_sub_agent, get_job_status,                  ││
│  │         get_job_output, get_job_artifacts, kill_job                     ││
│  └────────────────────────────────┬─────────────────────────────────────────┘│
│                                   │                                          │
│                                   ▼                                          │
│  ┌─────────────────────────────────────────────────────────────────────────┐│
│  │                    MCP Server (runs locally)                             ││
│  │                                                                          ││
│  │  - Holds FLASHPODS_API_TOKEN (never shared with containers)             ││
│  │  - Translates tool calls to HTTP API                                    ││
│  │  - Packages files via rsync                                             ││
│  │  - Handles artifact downloads                                           ││
│  └────────────────────────────────┬─────────────────────────────────────────┘│
│                                   │                                          │
│  ┌────────────────────────────────▼─────────────────────────────────────────┐│
│  │                    WireGuard Client (10.0.0.2)                           ││
│  └────────────────────────────────┬─────────────────────────────────────────┘│
└───────────────────────────────────┼──────────────────────────────────────────┘
                                    │
                                    │ Encrypted UDP (port 51820)
                                    │
┌───────────────────────────────────▼──────────────────────────────────────────┐
│                            FLASHPODS HOST                                     │
│                    (Hetzner NixOS - 8+ vCPU, 16GB+ RAM)                      │
│                                                                               │
│  ════════════════════════════ TRUST BOUNDARY ════════════════════════════   │
│                                                                               │
│  ┌─────────────────────────────────────────────────────────────────────────┐ │
│  │                    WireGuard Server (10.0.0.1)                           │ │
│  │                    Only port open to internet: 51820                     │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
│                                    │                                          │
│            ┌───────────────────────┴───────────────────────┐                 │
│            │                                               │                 │
│            ▼                                               ▼                 │
│  ┌──────────────────┐                          ┌──────────────────┐         │
│  │  rsync daemon    │                          │  Flashpods API   │         │
│  │  :873            │                          │  :8080           │         │
│  │                  │                          │                  │         │
│  │  Allowed: laptop │                          │  Auth: Bearer    │         │
│  │  Blocked: pods   │                          │  Allowed: laptop │         │
│  └──────────────────┘                          │  Blocked: pods   │         │
│                                                └────────┬─────────┘         │
│                                                         │                    │
│  ════════════════════════ CONTAINER BOUNDARY ═══════════│════════════════   │
│                                                         │                    │
│  ┌──────────────────────────────────────────────────────┼──────────────────┐│
│  │                    Podman (Rootless, user: flashpods)│                   ││
│  │                                                      │                   ││
│  │   ┌─────────────┐  ┌─────────────┐  ┌─────────────┐ │                   ││
│  │   │ job_abc123  │  │ job_def456  │  │ job_ghi789  │ │                   ││
│  │   │ (worker)    │  │ (worker)    │  │ (sub-agent) │ │                   ││
│  │   │             │  │             │  │             │ │                   ││
│  │   │ ✗ No API    │  │ ✗ No API    │  │ ✗ No API    │◄┘ Creates          ││
│  │   │ ✗ No rsync  │  │ ✗ No rsync  │  │ ✗ No rsync  │                    ││
│  │   │ ✓ SPIRE     │  │ ✓ SPIRE     │  │ ✓ SPIRE     │                    ││
│  │   │ ✓ Token svc │  │ ✓ Token svc │  │ ✓ Token svc │                    ││
│  │   │ ✓ Internet  │  │ ✓ Internet  │  │ ✓ Internet  │                    ││
│  │   └─────────────┘  └─────────────┘  └─────────────┘                    ││
│  │                                                                          ││
│  └──────────────────────────────────────────────────────────────────────────┘│
│                                                                               │
│  ┌─────────────────────────────────────────────────────────────────────────┐ │
│  │                         SPIRE                                            │ │
│  │                                                                          │ │
│  │  Server: CA + Registry                                                   │ │
│  │  Agent: Workload attestation via kernel cgroups                         │ │
│  │  Socket: /run/spire/sockets/agent.sock                                  │ │
│  │                                                                          │ │
│  │  Attestation: container ID from /proc/<pid>/cgroup → Podman metadata    │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
│                                                                               │
│  ┌─────────────────────────────────────────────────────────────────────────┐ │
│  │                         Token Service                                    │ │
│  │                                                                          │ │
│  │  Socket: /run/flashpods/token.sock (unix only, no TCP)                  │ │
│  │  Auth: JWT-SVID verification                                            │ │
│  │  Maps: SPIFFE ID → API tokens                                           │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
│                                                                               │
│  ┌─────────────────────────────────────────────────────────────────────────┐ │
│  │                         Storage                                          │ │
│  │                                                                          │ │
│  │   /tmp/flashpods/uploads/{upload_id}/     ← Temp, deleted after start   │ │
│  │   /var/lib/flashpods/artifacts/{job_id}/  ← TTL-only retention (1hr)   │ │
│  │   /var/lib/flashpods/db/flashpods.db      ← Job metadata                │ │
│  │   /var/log/flashpods/{job_id}.log         ← TTL-only retention (24hr)  │ │
│  │                                                                          │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────────────────┘
```

---

## Trust Boundary

### Authoritative Trust Model

| Source | Can Access API | Can Access rsync | Can Access Token Svc | Can Access Internet |
|--------|---------------|------------------|---------------------|---------------------|
| MCP Server (laptop) | ✅ Yes (bearer token) | ✅ Yes (IP allowlist) | N/A | N/A |
| Job containers | ❌ No (firewall) | ❌ No (firewall) | ✅ Yes (unix socket) | ✅ Yes |

### Invariant (System-Enforced)

> **Job containers can never call `POST /jobs` or access rsync.**
> 
> This is enforced by host firewall, not by convention.

This invariant guarantees that sub-agents cannot spawn more agents, regardless of what code runs inside them.

---

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Execution model** | Ephemeral containers | Simple, secure, no state leakage |
| **File upload** | rsync daemon (no SSH) | Delta sync efficiency, no static keys |
| **API auth** | Bearer token (MCP only) | Defense in depth beyond WireGuard |
| **Container runtime** | Podman rootless | Security without root |
| **Identity** | SPIRE (kernel cgroups) | Cryptographic, unforgeable identity |
| **Secret delivery** | Token exchange via unix socket | Containers can't intercept |
| **Sub-agent recursion** | Impossible (firewall) | System-enforced, not convention |
| **Job retry** | Manual (agent decides) | Keep it simple |
| **Log access** | Polling | Simpler than WebSocket |
| **Podman user** | `flashpods` with `--userns=keep-id` | Stable cgroup layout, socket permissions |

---

## Usage Patterns

### Pattern 1: Offload a Build

```
Agent: spawn_worker({
         command: "cargo build --release",
         files: { local_path: "/Users/me/myapp" }
       })
     ← job_id: "job_abc123"

Agent: *continues other work*

Agent: get_job_status("job_abc123")
     ← { status: "completed", exit_code: 0, duration: "8m 32s" }

Agent: get_job_artifacts("job_abc123")
     ← [ "myapp (4.2 MB)", "build.log" ]
```

### Pattern 2: Delegate to Sub-Agent

```
Agent: spawn_sub_agent({
         task: "Refactor auth module to use JWT",
         context: "Current code uses sessions...",
         files: { local_path: "/Users/me/myapp" },
         git_branch: "feature/jwt-auth"
       })
     ← job_id: "job_xyz789"

Agent: *continues other work for 30 minutes*

Agent: get_job_status("job_xyz789")
     ← { status: "completed", exit_code: 0 }

Agent: get_job_artifacts("job_xyz789")
     ← [ "summary.md", "changes.patch" ]

Agent: *reviews branch on GitHub, merges*
```

### Pattern 3: Parallel Test Runs

```
Agent: spawn_worker({ command: "npm test", files: {...}, image: "node:18" })
     ← job_id: "job_test_18"

Agent: spawn_worker({ command: "npm test", files: {...}, image: "node:20" })
     ← job_id: "job_test_20"

Agent: *both run in parallel*

Agent: get_job_status("job_test_18") → completed, exit_code: 0
Agent: get_job_status("job_test_20") → failed, exit_code: 1

Agent: get_job_output("job_test_20", tail: 20)
     ← "Error: X is not defined in Node 20..."
```

---

## Job Lifecycle

```
┌─────────┐     ┌──────────┐     ┌─────────┐     ┌───────────┐     ┌──────────┐     ┌─────────┐
│ pending │ ──► │ starting │ ──► │ running │ ──► │ completed │ ──► │ cleaning │ ──► │ cleaned │
└─────────┘     └──────────┘     └─────────┘     │ failed    │     └──────────┘     └─────────┘
                                                 │ timed_out │
                                                 │ cancelled │
                                                 └───────────┘

pending:    Job created, waiting for upload finalization
starting:   Container being created
running:    Command executing
completed:  Exit code 0
failed:     Exit code != 0, or container missing on reconciliation
timed_out:  Exceeded timeout_minutes
cancelled:  Killed via API
cleaning:   Logs and artifacts being deleted
cleaned:    Fully cleaned up, immutable
```

---

## Job Semantics

Flashpods jobs are **best-effort, non-resumable execution units** with **idempotent creation semantics**.

### Core Principles

1. **Best-effort:** Jobs may fail due to resource limits, timeouts, or infrastructure issues
2. **Non-resumable:** A failed/cancelled job cannot be resumed; create a new job instead
3. **Idempotent creation:** Same `client_job_id` returns existing job, not duplicate
4. **No automatic retries:** All retries are explicit and initiated by MCP

### Idempotency Key

**API change:** `POST /jobs` accepts an optional idempotency key:

```json
{
  "client_job_id": "550e8400-e29b-41d4-a716-446655440000",
  "type": "worker",
  "command": "cargo build --release",
  "files_id": "upload_abc123"
}
```

**Idempotency rule:**

If a job with the same `client_job_id` already exists and is **not** in state `cleaned`:
- API returns the existing job (HTTP 200, not 201)
- No new job is created
- No side effects

If job is in state `cleaned`:
- API creates a new job (old one is immutable history)

**This prevents:**
- Double builds on network retry
- Race conditions on MCP retries
- MCP crashes causing duplicate work
- Wasted compute on transient failures

**Response indicates whether job was created or retrieved:**

```json
{
  "job_id": "job_xyz789",
  "status": "running",
  "created": false,
  "message": "Existing job returned (idempotent)"
}
```

### Cancellation Semantics

**`kill_job(job_id)` behavior:**

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐     ┌───────────┐
│   SIGTERM   │ ──► │   grace     │ ──► │   SIGKILL   │ ──► │ cancelled │
│   sent      │     │   period    │     │   (if needed)│     │           │
└─────────────┘     │   (10s)     │     └─────────────┘     └───────────┘
                    └─────────────┘
```

1. Send `SIGTERM` to container main process
2. Wait grace period (default: 10 seconds, configurable)
3. If still running, send `SIGKILL`
4. Mark job `cancelled`

**Cancellation invariant:**

A cancelled job:
- **Never** transitions to `completed`
- Retains logs/artifacts until TTL cleanup
- Is **not resumable**
- Cannot be un-cancelled

### Timeout Semantics

If runtime exceeds `timeout_minutes`:
1. Container is killed (same process as cancellation)
2. Job marked `timed_out`

**Timeout invariant:**

A timed_out job:
- Is equivalent to `cancelled` for cleanup purposes
- Retains partial logs/artifacts until TTL
- Is **not retried automatically**
- Records actual runtime for debugging

```json
{
  "id": "job_xyz789",
  "status": "timed_out",
  "timeout_minutes": 30,
  "actual_runtime_seconds": 1800,
  "message": "Job exceeded timeout limit"
}
```

### Cleanup Semantics

Cleanup **only occurs** when job is in terminal state:

```
completed | failed | timed_out | cancelled
```

Cleanup transitions:

```
terminal state ──► cleaning ──► cleaned
```

| State | Logs | Artifacts | Job Record |
|-------|------|-----------|------------|
| completed/failed/timed_out/cancelled | Available | Available | Available |
| cleaning | Being deleted | Being deleted | Available |
| cleaned | Deleted | Deleted | Available (immutable) |

**After `cleaned`:**
- Job record remains in database (for history/debugging)
- Logs and artifacts are permanently deleted
- Job is immutable and unrecoverable
- Same `client_job_id` can create a new job

### API Crash Safety

If the Flashpods API crashes while jobs are in `starting` or `running` state:

**On restart, the API MUST:**

1. Enumerate all active containers with label `flashpods-job=true`
2. Query database for jobs in `starting` or `running` state
3. Reconcile:

| Container Exists | DB State | Action |
|------------------|----------|--------|
| Yes | starting/running | Update to `running`, continue monitoring |
| No | starting | Mark `failed`, error: "container_not_found_on_recovery" |
| No | running | Mark `failed`, error: "container_lost_on_recovery" |
| Yes | not in DB | Kill container (orphaned) |

**This avoids "ghost jobs"** - jobs that appear running but have no container.

```rust
async fn reconcile_on_startup() {
    // 1. Get all flashpods containers
    let containers = podman_list_containers("flashpods-job=true").await;
    let container_job_ids: HashSet<_> = containers
        .iter()
        .filter_map(|c| c.labels.get("flashpods-job-id"))
        .collect();
    
    // 2. Get all non-terminal jobs from DB
    let active_jobs = db.query("SELECT * FROM jobs WHERE status IN ('starting', 'running')").await;
    
    // 3. Reconcile
    for job in active_jobs {
        if container_job_ids.contains(&job.id) {
            // Container exists, ensure status is running
            db.update_status(&job.id, "running").await;
        } else {
            // Container missing, mark failed
            db.update_status(&job.id, "failed").await;
            db.set_error(&job.id, "container_lost_on_recovery").await;
        }
    }
    
    // 4. Kill orphaned containers (not in DB)
    for container in containers {
        let job_id = container.labels.get("flashpods-job-id");
        if !db.job_exists(job_id).await {
            podman_kill(&container.id).await;
        }
    }
}
```

### No Automatic Retries

**Flashpods never automatically retries jobs.**

All retries are:
- Explicit
- Initiated by MCP
- Create new jobs (with new or same `client_job_id`)

**This is critical for determinism:**
- MCP controls retry policy
- No hidden retry loops
- No unexpected resource consumption
- Failures are visible and debuggable

**MCP retry pattern:**

```typescript
async function spawnWorkerWithRetry(
  command: string, 
  files: FileSpec, 
  maxRetries: number = 3
): Promise<JobResult> {
  const clientJobId = crypto.randomUUID();
  
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    const job = await spawnWorker(command, files, { clientJobId });
    const result = await waitForJob(job.job_id);
    
    if (result.status === 'completed') {
      return result;
    }
    
    if (result.status === 'failed' && isRetryable(result.error)) {
      // Use same clientJobId - if job already exists, we get it back
      continue;
    }
    
    throw new Error(`Job failed: ${result.error}`);
  }
  
  throw new Error(`Job failed after ${maxRetries} attempts`);
}
```

---

## Resource Scheduling

Flashpods enforces **strict host-level resource limits** and does **not overcommit** CPU or memory.

### System Invariant

> **Flashpods never starts a job that would cause total allocated CPU or memory to exceed host capacity.**

This is the line that turns "best effort system" into "reliable system".

### Admission Control

When creating a job, the API **MUST:**

1. Compute available resources:

```
available_cpus = host_total_cpus - sum(running_job_cpus)
available_memory = host_total_memory - sum(running_job_memory)
```

2. If requested resources exceed availability:
   - Reject the job with `429 Insufficient Resources`
   - Include current capacity in response

**No silent degradation. No overcommit. No surprise OOM kills.**

### Rejection Response

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

### Queue Strategy: Reject (MVP)

**For MVP: No queue. Jobs are rejected when resources are unavailable.**

Rationale:
- MCP already handles retries via idempotency key
- Easier to reason about
- No background scheduler complexity
- Explicit failure is better than hidden waiting

**MCP handles rejection:**

```typescript
async function spawnWorkerWithBackoff(command: string, files: FileSpec): Promise<Job> {
  const clientJobId = crypto.randomUUID();
  
  for (let attempt = 0; attempt < 5; attempt++) {
    try {
      return await spawnWorker(command, files, { clientJobId });
    } catch (e) {
      if (e.status === 429) {
        // Resources unavailable, wait and retry
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

### No Preemption

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

### Resource Caps by Job Type

| Type | Max CPUs | Max Memory | Rationale |
|------|----------|------------|-----------|
| Worker | 8 | 16 GB | Full host capacity for heavy builds |
| Sub-agent | 4 | 8 GB | Capped to prevent runaway delegation |

**Sub-agent caps are enforced even if host has more capacity.**

This prevents:
- Runaway recursive resource consumption
- Accidental host saturation from many sub-agents
- Single sub-agent starving other jobs

### Resource Tracking

```sql
-- Query current resource usage
SELECT 
  SUM(cpus) as used_cpus,
  SUM(memory_gb) as used_memory_gb,
  COUNT(*) as running_jobs
FROM jobs 
WHERE status IN ('starting', 'running');
```

### Admission Control Implementation

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

### Future: FIFO Queue (Post-MVP)

If needed later, add `queued` state:

```
pending → queued → starting → running → ...
```

Jobs enter `queued` when resources unavailable, scheduler starts them FIFO when resources free up. Not in MVP.

---

## Component Specifications

### 1. Flashpods API (Rust)

**Tech Stack:**
- Rust 2021
- Axum (HTTP)
- SQLite (sqlx)
- Tokio (async)
- Podman CLI

**Authentication:**

All API requests require:
```
Authorization: Bearer <FLASHPODS_API_TOKEN>
```

Token is stored only in MCP server. Never injected into containers.

**Database Schema:**

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

-- Separate idempotency tracking table
CREATE TABLE idempotency_keys (
    client_job_id TEXT PRIMARY KEY,   -- Idempotency key from MCP
    job_id TEXT NOT NULL REFERENCES jobs(id),
    active INTEGER NOT NULL DEFAULT 1 -- 1 = active, 0 = job cleaned (key can be reused)
);

CREATE INDEX idx_idempotency_active ON idempotency_keys(client_job_id) WHERE active = 1;

CREATE TABLE artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES jobs(id),
    name TEXT NOT NULL,
    path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_artifacts_job_id ON artifacts(job_id);
```

**Idempotency Logic:**

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

### 2. MCP Server (TypeScript)

**Location:** Runs on your laptop, talks to Flashpods API.

**Responsibilities:**
- Store `FLASHPODS_API_TOKEN` (never shared)
- Translate MCP tool calls to HTTP API calls
- Execute rsync for file uploads
- Stream artifact downloads to local filesystem

### 3. Token Exchange Service (Rust)

**Transport:** Unix socket only (`/run/flashpods/token.sock`)

**No TCP listener.** This is critical for security.

**Purpose:** Validate SPIRE JWT-SVIDs, return API tokens based on SPIFFE ID.

---

## File Upload (rsync Daemon)

### Why rsync Daemon?

| Approach | SSH Keys | Delta Sync | Security |
|----------|----------|------------|----------|
| rsync + SSH | ❌ Needs keys | ✅ Yes | Static secret |
| rsync daemon | ✅ No keys | ✅ Yes | IP allowlist + firewall |
| HTTP upload | ✅ No keys | ❌ No | Simple |

**rsync daemon gives us delta sync without SSH keys.**

### Flow

```
┌─────────────────┐                      ┌─────────────────┐
│  MCP Server     │                      │  Flashpods Host │
│                 │                      │                 │
│  1. Generate    │                      │                 │
│     upload_id   │                      │                 │
│                 │                      │                 │
│  2. rsync ──────┼── WireGuard ─────────┼──► rsync daemon │
│     (delta)     │                      │     :873        │
│                 │                      │     ↓           │
│                 │                      │  /tmp/flashpods/│
│                 │                      │  uploads/       │
│                 │                      │  {upload_id}/   │
│                 │                      │                 │
│  3. POST        │                      │                 │
│     /uploads/   ┼──────────────────────┼──► API :8080    │
│     {id}/       │                      │     (finalize)  │
│     finalize    │                      │                 │
│                 │                      │                 │
│  4. POST /jobs ─┼──────────────────────┼──► API :8080    │
│     + Bearer    │                      │                 │
│     files_id:   │                      │                 │
│     upload_id   │                      │                 │
│                 │                      │                 │
└─────────────────┘                      └─────────────────┘
```

### rsync Command (MCP Server)

```typescript
async function uploadFiles(localPath: string, exclude: string[]): Promise<string> {
  const uploadId = `upload_${crypto.randomUUID().slice(0, 12)}`;
  
  const args = [
    '-avz',
    '--delete',
    ...exclude.flatMap(p => ['--exclude', p]),
    `${localPath}/`,
    `rsync://10.0.0.1/uploads/${uploadId}/`
  ];
  
  await execAsync('rsync', args);
  return uploadId;
}
```

---

## File Upload Lifecycle

Flashpods treats file uploads as first-class lifecycle objects.

### Upload States

```
┌───────────┐     ┌───────────┐     ┌──────────┐
│ uploading │ ──► │ finalized │ ──► │ consumed │
└───────────┘     └───────────┘     └──────────┘
      │                 │
      │ TTL expired     │ TTL expired (unused)
      ▼                 ▼
┌───────────┐     ┌───────────┐
│  expired  │     │  expired  │
└───────────┘     └───────────┘
```

| State | Description |
|-------|-------------|
| `uploading` | rsync in progress or complete, not yet finalized |
| `finalized` | Explicitly finalized by MCP, eligible for job creation |
| `consumed` | Referenced by a job that reached `running` state |
| `expired` | TTL exceeded, marked for deletion |

### Finalization Step

Jobs can only reference uploads in state `finalized`. MCP must call:

```http
POST /uploads/{id}/finalize
Authorization: Bearer <token>
```

**Response:**
```json
{
  "upload_id": "upload_abc123",
  "state": "finalized",
  "size_bytes": 15728640,
  "file_count": 847,
  "finalized_at": "2026-01-21T10:30:00Z",
  "expires_at": "2026-01-21T11:00:00Z"
}
```

This provides **atomicity**: the API knows the upload is complete and intact before any job can use it.

**Error cases:**
- Upload doesn't exist → `404 Not Found`
- Upload already finalized → `409 Conflict`
- Upload already consumed → `409 Conflict`
- Upload expired → `410 Gone`

### Upload Quotas

| Constraint | Default | Configurable |
|------------|---------|--------------|
| Max upload size | 2 GB | Yes |
| Max total upload disk usage | 10 GB | Yes |
| Max upload TTL (non-finalized) | 30 minutes | Yes |
| Max upload TTL (finalized, unused) | 60 minutes | Yes |

When quotas are exceeded:
- New rsync uploads fail with disk full error
- API returns `507 Insufficient Storage`

### Cleanup Rules

| Condition | Action |
|-----------|--------|
| Non-finalized upload exceeds TTL | Delete immediately |
| Finalized but unused upload exceeds TTL | Delete immediately |
| Upload consumed (job reached `running`) | Delete immediately after container start |
| Job fails before `running` state | Delete upload immediately |
| Job cancelled before `running` state | Delete upload immediately |

### Integrity Invariant

> **Container sees an immutable snapshot of a finalized upload.**

Guarantees:
1. Upload directory is mounted **read-only** into container
2. Upload state must be `finalized` before job creation
3. Upload directory is deleted **only after**:
   - Container successfully starts (state → `consumed`), OR
   - Job is cancelled/fails before `running`
4. No concurrent modification of upload directory once finalized

### Database Schema (Uploads)

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

### API Endpoints (Uploads)

| Method | Path | Description |
|--------|------|-------------|
| POST | /uploads/{id}/finalize | Mark upload as finalized |
| GET | /uploads/{id} | Get upload status |
| DELETE | /uploads/{id} | Cancel/delete upload |

### MCP Flow Update

```typescript
async function spawnWorker(command: string, files: FileSpec): Promise<string> {
  // 1. Generate upload ID
  const uploadId = `upload_${crypto.randomUUID().slice(0, 12)}`;
  
  // 2. rsync files
  await rsyncFiles(files.local_path, uploadId, files.exclude);
  
  // 3. Finalize upload (NEW - required step)
  await fetch(`http://10.0.0.1:8080/uploads/${uploadId}/finalize`, {
    method: 'POST',
    headers: { 'Authorization': `Bearer ${API_TOKEN}` }
  });
  
  // 4. Create job referencing finalized upload
  const response = await fetch('http://10.0.0.1:8080/jobs', {
    method: 'POST',
    headers: { 
      'Authorization': `Bearer ${API_TOKEN}`,
      'Content-Type': 'application/json'
    },
    body: JSON.stringify({
      type: 'worker',
      command,
      files_id: uploadId,  // Must be finalized
      // ...
    })
  });
  
  return response.job_id;
}
```

### Cleanup Daemon

```rust
// Runs every minute
async fn cleanup_expired_uploads() {
    let now = Utc::now();
    
    // Delete expired uploads
    let expired = sqlx::query!(
        "SELECT id, state FROM uploads WHERE expires_at < ? AND state IN ('uploading', 'finalized')",
        now
    ).fetch_all(&pool).await?;
    
    for upload in expired {
        // Delete from filesystem
        fs::remove_dir_all(format!("/tmp/flashpods/uploads/{}", upload.id))?;
        
        // Update state
        sqlx::query!(
            "UPDATE uploads SET state = 'expired' WHERE id = ?",
            upload.id
        ).execute(&pool).await?;
    }
}
```

---

## Artifact Handling

---

## Logs

### Log Capture Model

Each job writes stdout/stderr to a server-side log file:

```
/var/log/flashpods/{job_id}.log
```

Logs are captured in real-time as the container runs.

### Log Limits

| Constraint | Default | Configurable |
|------------|---------|--------------|
| Max log size per job | 50 MB | Yes |
| Max total log disk usage | 10 GB | Yes |
| Log retention TTL (completed jobs) | 24 hours | Yes |

### Truncation Semantics

If log exceeds max size:
1. Flashpods **stops capturing further output**
2. Job **continues running** (not killed)
3. Log is marked `truncated=true`
4. Final log contains first N bytes up to limit

### API Contract

**GET /jobs/:id/output**

```json
{
  "output": "Compiling myapp v0.1.0\n...",
  "lines": 100,
  "truncated": true,
  "total_bytes": 52428800
}
```

| Field | Description |
|-------|-------------|
| `output` | Log content (tail N lines by default) |
| `lines` | Number of lines returned |
| `truncated` | True if log exceeded max size |
| `total_bytes` | Total size of log file |

### Log Cleanup

| Condition | Action |
|-----------|--------|
| Job completed + TTL exceeded | Delete log file |
| Job failed + TTL exceeded | Delete log file |
| Job timed_out + TTL exceeded | Delete log file |
| Job cancelled + TTL exceeded | Delete log file |
| Total disk usage exceeded | Delete oldest completed job logs first |

---

## Artifacts

### Artifact Write Rules

Jobs write outputs **only** to `/artifacts` inside the container.

```bash
# Inside container
cp target/release/myapp /artifacts/
echo "Build complete" > /artifacts/summary.txt
```

### Server-Side Storage

Artifacts are copied to:

```
/var/lib/flashpods/artifacts/{job_id}/
```

### Artifact Constraints

| Constraint | Default | Configurable |
|------------|---------|--------------|
| Max artifact size per file | 1 GB | Yes |
| Max total artifacts per job | 2 GB | Yes |
| Max number of artifacts per job | 200 | Yes |
| Artifact retention TTL | 1 hour | Yes |

### Overwrite Semantics

**Artifact names must be unique.**

If an artifact name already exists within the same job, the job fails with:

```json
{
  "error": "artifact_name_conflict",
  "message": "Artifact 'output.txt' already exists"
}
```

**No silent overwrites.** This prevents accidental data loss and makes behavior predictable.

### Artifact Name Safety (Mandatory)

Artifact names are treated as **filenames, not paths**.

The server **MUST reject** artifact names containing:
- `/` or `\` (path separators)
- `..` (parent directory traversal)
- NUL bytes (`\0`)
- Leading or trailing whitespace
- Empty string

**Valid:** `myapp`, `build.log`, `report-2026-01-21.pdf`

**Invalid:** `../../../etc/passwd`, `foo/bar.txt`, ``, `  `

Artifact downloads **MUST resolve strictly within:**

```
/var/lib/flashpods/artifacts/{job_id}/
```

This removes an entire class of path traversal vulnerabilities.

### Artifact Validation

```rust
fn validate_artifact_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::InvalidArtifactName("empty name"));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(Error::InvalidArtifactName("contains path separator"));
    }
    if name.contains("..") {
        return Err(Error::InvalidArtifactName("contains parent traversal"));
    }
    if name.contains('\0') {
        return Err(Error::InvalidArtifactName("contains NUL byte"));
    }
    if name.trim() != name {
        return Err(Error::InvalidArtifactName("leading/trailing whitespace"));
    }
    Ok(())
}
```

### Cleanup Semantics

Cleanup policy:
- Artifacts are deleted after TTL, **regardless of download status**
- Partial downloads do not affect cleanup
- Jobs are marked `cleaned_up` once logs and artifacts are deleted

**No race conditions. No hidden state. No "did the client finish downloading?".**

### Artifact Lifecycle

```
┌──────────┐     ┌───────────┐     ┌────────────┐
│ created  │ ──► │ available │ ──► │  deleted   │
└──────────┘     └───────────┘     └────────────┘
                       │
                       │ TTL expired
                       ▼
                 ┌───────────┐
                 │  deleted  │
                 └───────────┘
```

| State | Description |
|-------|-------------|
| `created` | Container wrote file to /artifacts |
| `available` | Job completed, artifact accessible via API |
| `deleted` | TTL expired or job cancelled |

### API Endpoints (Artifacts)

**GET /jobs/:id/artifacts**

```json
{
  "artifacts": [
    {
      "name": "myapp",
      "size_bytes": 4404224,
      "created_at": "2026-01-21T10:35:00Z"
    },
    {
      "name": "build.log",
      "size_bytes": 131072,
      "created_at": "2026-01-21T10:35:00Z"
    }
  ],
  "total_size_bytes": 4535296,
  "expires_at": "2026-01-21T11:35:00Z"
}
```

**GET /jobs/:id/artifacts/:name**

Returns artifact as binary stream with appropriate Content-Type.

```http
HTTP/1.1 200 OK
Content-Type: application/octet-stream
Content-Length: 4404224
Content-Disposition: attachment; filename="myapp"

<binary data>
```

### Database Schema (Artifacts)

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

---

## Identity & Secrets (SPIRE)

Flashpods uses SPIRE for cryptographic workload identity.

### Workload Attestation Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SPIRE Attestation                                  │
│                                                                              │
│  1. Container connects to SPIRE Workload API socket:                        │
│     /run/spire/sockets/agent.sock                                           │
│                                                                              │
│  2. SPIRE Agent:                                                            │
│     a. Receives request                                                     │
│     b. Obtains caller PID from socket                                       │
│     c. Reads /proc/<pid>/cgroup                                             │
│     d. Extracts container ID from cgroup path                               │
│     e. Queries Podman (docker-compatible API) for container metadata        │
│     f. Converts metadata to selectors                                       │
│                                                                              │
│  3. Selectors generated (example):                                          │
│     - docker:label:flashpods-job-type:agent                                 │
│                                                                              │
│  4. Selectors matched against registration entries:                         │
│     - spiffe://flashpods.local/agent                                        │
│     - spiffe://flashpods.local/worker                                       │
│                                                                              │
│  5. If matched, SPIRE issues SVID (JWT or X.509) to the container          │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Critical Point

> **Containers never claim identity.**
> 
> Identity is derived from kernel cgroup metadata + runtime labels only.
> This is unforgeable from inside the container.

### SPIRE Registration

```bash
# Register agent jobs
spire-server entry create \
    -spiffeID spiffe://flashpods.local/agent \
    -parentID spiffe://flashpods.local/host \
    -selector docker:label:flashpods-job-type:agent

# Register worker jobs
spire-server entry create \
    -spiffeID spiffe://flashpods.local/worker \
    -parentID spiffe://flashpods.local/host \
    -selector docker:label:flashpods-job-type:worker
```

### Workload Attestation Requirements

SPIRE Agent must be configured with:

1. **Docker workload attestor** (works with Podman)
2. **Access to Podman docker-compatible socket:**
   ```
   /run/flashpods/podman.sock
   ```
3. **Cgroup parsing configuration:**
   ```
   cgroup_prefix
   cgroup_container_index
   ```

These values must be derived from actual cgroup paths on the host:
```bash
cat /proc/<container_pid>/cgroup
```

This configuration is host-specific and must be validated during deployment.

---

## Token Exchange Service

### Architecture

Token Service listens **only** on unix socket:
```
/run/flashpods/token.sock
```

**No TCP listener.** Containers access it via mounted socket only.

### Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Token Exchange Flow                                │
│                                                                              │
│  1. Container fetches JWT-SVID from SPIRE:                                  │
│                                                                              │
│     spire-agent api fetch jwt \                                             │
│       -socketPath /run/spire/sockets/agent.sock \                           │
│       -audience flashpods-token-service                                     │
│                                                                              │
│  2. Container calls token service via unix socket:                          │
│                                                                              │
│     curl --unix-socket /run/flashpods/token.sock \                          │
│       -H "Authorization: Bearer <JWT-SVID>" \                               │
│       http://localhost/tokens                                                │
│                                                                              │
│  3. Token service:                                                          │
│     a. Verifies JWT signature (SPIRE trust bundle)                          │
│     b. Verifies audience = "flashpods-token-service"                        │
│     c. Extracts SPIFFE ID from JWT subject                                  │
│     d. Maps SPIFFE ID to token scope                                        │
│     e. Returns tokens                                                       │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Token Policy

| SPIFFE ID | ANTHROPIC_API_KEY | GITHUB_TOKEN |
|-----------|-------------------|--------------|
| `spiffe://flashpods.local/agent` | ✅ Yes | ✅ Yes |
| `spiffe://flashpods.local/worker` | ❌ No | ✅ Yes |

**Token service ignores any caller-supplied job type.** Only the cryptographically-verified SPIFFE ID matters.

### Token Service Implementation

```rust
// Listens on /run/flashpods/token.sock

async fn handle_request(auth_header: &str) -> Result<Tokens, Error> {
    // 1. Extract JWT from "Bearer <jwt>"
    let jwt = auth_header.strip_prefix("Bearer ")
        .ok_or(Error::MissingBearer)?;
    
    // 2. Verify JWT signature using SPIRE trust bundle
    let claims = verify_jwt(jwt, &spire_trust_bundle)?;
    
    // 3. Check audience
    if !claims.aud.contains("flashpods-token-service") {
        return Err(Error::InvalidAudience);
    }
    
    // 4. Extract SPIFFE ID and return appropriate tokens
    match claims.sub.as_str() {
        "spiffe://flashpods.local/agent" => Ok(Tokens {
            anthropic_api_key: Some(read_secret("anthropic")),
            github_token: Some(read_secret("github")),
        }),
        "spiffe://flashpods.local/worker" => Ok(Tokens {
            anthropic_api_key: None,
            github_token: Some(read_secret("github")),
        }),
        _ => Err(Error::UnknownIdentity),
    }
}
```

### Container Entrypoint (Sub-Agent)

```bash
#!/bin/bash
set -e

# 1. Fetch JWT-SVID from SPIRE
JWT=$(spire-agent api fetch jwt \
    -socketPath /run/spire/sockets/agent.sock \
    -audience flashpods-token-service \
    | jq -r '.[] | .svids[0].svid')

# 2. Exchange JWT for API tokens via unix socket
TOKENS=$(curl -s --unix-socket /run/flashpods/token.sock \
    -H "Authorization: Bearer $JWT" \
    http://localhost/tokens)

export ANTHROPIC_API_KEY=$(echo $TOKENS | jq -r '.ANTHROPIC_API_KEY')
export GITHUB_TOKEN=$(echo $TOKENS | jq -r '.GITHUB_TOKEN')

# 3. Setup git
cd /work
git config user.email "flashpods@local"
git config user.name "Flashpods Agent"
git checkout -b "$FLASHPODS_GIT_BRANCH"

# 4. Run Claude with task
claude --task "$FLASHPODS_TASK" --context "$FLASHPODS_CONTEXT" --workdir /work

# 5. Push and summarize
git push origin "$FLASHPODS_GIT_BRANCH"
git diff main > /artifacts/changes.patch
echo "Task completed. See branch: $FLASHPODS_GIT_BRANCH" > /artifacts/summary.md
```

---

## MCP Tools

### spawn_worker

```typescript
{
  name: "spawn_worker",
  description: `Spawn a container to run a command (build, test, script).
Returns immediately with job ID. Job runs asynchronously.
Use get_job_status to check progress, get_job_output for logs.

Container gets:
- Your files at /work (read-only)
- /artifacts for outputs (write here)
- GitHub token for git operations

Examples: "cargo build --release", "npm test", "pytest -x"`,

  inputSchema: {
    type: "object",
    properties: {
      command: {
        type: "string",
        description: "Shell command to execute"
      },
      files: {
        type: "object",
        properties: {
          local_path: {
            type: "string",
            description: "Local directory to upload (e.g., /Users/me/myapp)"
          },
          exclude: {
            type: "array",
            items: { type: "string" },
            default: [".git", "node_modules", "target", "__pycache__", ".venv"]
          }
        },
        required: ["local_path"]
      },
      image: {
        type: "string",
        default: "ubuntu:22.04"
      },
      cpus: {
        type: "integer",
        default: 2,
        maximum: 8
      },
      memory_gb: {
        type: "integer",
        default: 4,
        maximum: 16
      },
      timeout_minutes: {
        type: "integer",
        default: 30,
        maximum: 120
      }
    },
    required: ["command"]
  }
}
```

### spawn_sub_agent

```typescript
{
  name: "spawn_sub_agent",
  description: `Spawn another Claude instance to work on a task autonomously.
Returns job ID. Sub-agent works independently in its own container.

Sub-agent will:
1. Receive your files at /work (read-write)
2. Work on the specified git branch
3. Complete the task autonomously
4. Push changes to remote
5. Write summary to /artifacts

Sub-agents CANNOT spawn other agents or workers (enforced by firewall).
Use for: refactoring, implementing features, fixing bugs, writing tests.`,

  inputSchema: {
    type: "object",
    properties: {
      task: {
        type: "string",
        description: "Clear description of what to accomplish"
      },
      context: {
        type: "string",
        description: "Relevant context, code snippets, requirements"
      },
      files: {
        type: "object",
        properties: {
          local_path: { type: "string" },
          exclude: { type: "array", items: { type: "string" } }
        },
        required: ["local_path"]
      },
      git_branch: {
        type: "string",
        description: "Branch for sub-agent to work on (required)"
      },
      timeout_minutes: {
        type: "integer",
        default: 60,
        maximum: 120
      }
    },
    required: ["task", "git_branch"]
  }
}
```

### get_job_status

```typescript
{
  name: "get_job_status",
  description: "Check job status. Returns: status, elapsed time, exit code if complete.",
  inputSchema: {
    type: "object",
    properties: {
      job_id: { type: "string" }
    },
    required: ["job_id"]
  }
}
```

### get_job_output

```typescript
{
  name: "get_job_output",
  description: "Get stdout/stderr from a job. Works while running or after completion.",
  inputSchema: {
    type: "object",
    properties: {
      job_id: { type: "string" },
      tail: {
        type: "integer",
        default: 100,
        description: "Number of lines from end"
      }
    },
    required: ["job_id"]
  }
}
```

### get_job_artifacts

```typescript
{
  name: "get_job_artifacts",
  description: "List artifacts from a completed job.",
  inputSchema: {
    type: "object",
    properties: {
      job_id: { type: "string" }
    },
    required: ["job_id"]
  }
}
```

### download_artifact

```typescript
{
  name: "download_artifact",
  description: "Download an artifact to local filesystem.",
  inputSchema: {
    type: "object",
    properties: {
      job_id: { type: "string" },
      artifact_name: { type: "string" },
      save_to: {
        type: "string",
        description: "Local path to save file"
      }
    },
    required: ["job_id", "artifact_name"]
  }
}
```

### list_jobs

```typescript
{
  name: "list_jobs",
  description: "List jobs, optionally filtered by status.",
  inputSchema: {
    type: "object",
    properties: {
      status: {
        type: "string",
        enum: ["all", "running", "completed", "failed"]
      },
      limit: {
        type: "integer",
        default: 20
      }
    }
  }
}
```

### kill_job

```typescript
{
  name: "kill_job",
  description: "Terminate a running job immediately.",
  inputSchema: {
    type: "object",
    properties: {
      job_id: { type: "string" }
    },
    required: ["job_id"]
  }
}
```

---

## API Specification

### Authentication

All requests require:
```
Authorization: Bearer <FLASHPODS_API_TOKEN>
```

Requests without valid token receive `401 Unauthorized`.

### Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | /uploads/{id}/finalize | Finalize upload |
| GET | /uploads/{id} | Get upload status |
| DELETE | /uploads/{id} | Cancel/delete upload |
| POST | /jobs | Create job |
| GET | /jobs | List jobs |
| GET | /jobs/:id | Get job details |
| GET | /jobs/:id/output | Get stdout/stderr |
| GET | /jobs/:id/artifacts | List artifacts |
| GET | /jobs/:id/artifacts/:name | Download artifact |
| DELETE | /jobs/:id | Kill job |
| GET | /health | Health check (no auth) |

### POST /uploads/{id}/finalize

**Request:**
```http
POST /uploads/upload_abc123/finalize HTTP/1.1
Authorization: Bearer <token>
```

**Response:**
```json
{
  "upload_id": "upload_abc123",
  "state": "finalized",
  "size_bytes": 15728640,
  "file_count": 847,
  "finalized_at": "2026-01-21T10:30:00Z",
  "expires_at": "2026-01-21T11:00:00Z"
}
```

### POST /jobs

**Request:**
```http
POST /jobs HTTP/1.1
Authorization: Bearer <token>
Content-Type: application/json

{
  "client_job_id": "550e8400-e29b-41d4-a716-446655440000",
  "type": "worker",
  "command": "cargo build --release",
  "files_id": "upload_abc123",
  "image": "rust:latest",
  "cpus": 4,
  "memory_gb": 8,
  "timeout_minutes": 30
}
```

**Response (new job created):**
```json
{
  "job_id": "job_xyz789",
  "status": "starting",
  "created": true
}
```

**Response (existing job returned - idempotent):**
```json
{
  "job_id": "job_xyz789",
  "status": "running",
  "created": false,
  "message": "Existing job returned (idempotent)"
}
```

### GET /jobs/:id

**Response:**
```json
{
  "id": "job_xyz789",
  "type": "worker",
  "status": "running",
  "command": "cargo build --release",
  "image": "rust:latest",
  "created_at": "2026-01-18T10:30:00Z",
  "started_at": "2026-01-18T10:30:02Z",
  "elapsed_seconds": 145
}
```

### GET /jobs/:id/output

**Query params:** `?tail=100`

**Response:**
```json
{
  "output": "Compiling myapp v0.1.0\n...",
  "lines": 100,
  "truncated": true,
  "total_bytes": 52428800
}
```

| Field | Description |
|-------|-------------|
| `output` | Log content (tail N lines) |
| `lines` | Number of lines returned |
| `truncated` | True if log exceeded max size limit |
| `total_bytes` | Total size of log file |

---

## Security Model

### MVP Security Model (System-Enforced)

| Layer | Mechanism |
|-------|-----------|
| Network | WireGuard |
| API access | Bearer token (MCP only) |
| Job spawning | Firewall blocks containers from API |
| File upload | rsyncd allowlist = laptop IP only |
| Identity | SPIRE workload attestation (kernel cgroups) |
| Secrets | Token exchange via SPIFFE ID |
| Container isolation | Rootless Podman |
| Privilege | Worker vs Agent via SPIFFE ID |

### Firewall Policy

| Port | Allowed Source |
|------|----------------|
| 51820 (WireGuard) | Internet |
| 8080 (API) | 10.0.0.2 only |
| 873 (rsync) | 10.0.0.2 only |
| token service | Unix socket only |

**Containers are blocked from 8080 and 873 by host firewall.**

### Container Network Access

| Destination | Allowed |
|-------------|---------|
| Internet (Anthropic, GitHub, npm) | ✅ Yes |
| API (10.0.0.1:8080) | ❌ No (firewall) |
| rsync (10.0.0.1:873) | ❌ No (firewall) |
| SPIRE socket | ✅ Yes (mounted) |
| Token socket | ✅ Yes (mounted) |

### Podman Execution Invariant

All containers are launched:
- **Rootless** (no root inside or outside container)
- **As host user `flashpods`**
- **With `--userns=keep-id`**

This ensures:
- Socket permissions work correctly
- Image storage is shared
- Cgroup layout is stable (required for SPIRE attestation)

### Container Mounts

| Path | Purpose | Worker Mode | Sub-Agent Mode |
|------|---------|-------------|----------------|
| /work | Job files | ro | rw |
| /artifacts | Outputs | rw | rw |
| /run/spire/sockets/agent.sock | SPIRE Workload API | ro | ro |
| /run/flashpods/token.sock | Token exchange | ro | ro |

### /work Mount Semantics

Flashpods enforces different `/work` mount modes based on job type.

| Job Type | /work Mode | Semantics |
|----------|------------|-----------|
| Worker | `ro` | Immutable input snapshot |
| Sub-agent | `rw` | Draft workspace |

**Workers:**
- Receive a **read-only** view of uploaded files
- Cannot mutate source code
- All outputs must go through `/artifacts`

This guarantees:
- Deterministic execution
- Reproducible builds/tests
- No hidden side effects

**Sub-agents:**
- Receive a **read-write** workspace
- May mutate files locally
- Must publish all authoritative changes via git commits

Local filesystem changes inside sub-agent containers are **non-authoritative** and become real only after passing through:

```
git commit → git push → review → merge
```

### System Guarantee

> The source of truth for code changes is always git, never the container filesystem.

Sub-agent `/work` mutations are ephemeral drafts. Without `git push`, they disappear when the container dies.

---

## NixOS Configuration

### flake.nix (Structure)

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
  };

  outputs = { self, nixpkgs }: {
    nixosConfigurations.flashpods = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        ./modules/wireguard.nix
        ./modules/firewall.nix
        ./modules/podman.nix
        ./modules/rsyncd.nix
        ./modules/spire.nix
        ./modules/flashpods-api.nix
        ./modules/token-service.nix
      ];
    };
  };
}
```

### Firewall (Critical)

```nix
# modules/firewall.nix
{ ... }: {
  # Use nftables backend (NixOS default on newer versions)
  networking.nftables.enable = true;
  
  networking.firewall = {
    enable = true;
    
    # WireGuard from internet
    allowedUDPPorts = [ 51820 ];
  };

  # Custom nftables rules for Flashpods
  networking.nftables.tables.flashpods = {
    family = "inet";
    content = ''
      chain input {
        type filter hook input priority 0; policy accept;
        
        # Allow API and rsync only from WireGuard peer (laptop)
        iifname "wg0" tcp dport 8080 ip saddr 10.0.0.2 accept
        iifname "wg0" tcp dport 873 ip saddr 10.0.0.2 accept
        
        # Block API and rsync from Podman bridge interfaces
        # This matches any podman-created bridge (podman0, cni-podman*, etc.)
        iifname "podman*" tcp dport { 8080, 873 } drop
        iifname "cni-*" tcp dport { 8080, 873 } drop
        
        # Explicit drop for API and rsync from anywhere else
        tcp dport { 8080, 873 } drop
      }
    '';
  };
}
```

**Why nftables + interface matching:**
- nftables is the modern Linux firewall (iptables successor)
- NixOS nftables integration is declarative and deterministic
- Interface matching (`iifname "podman*"`) works regardless of container subnet
- No hardcoded CIDRs that may change between Podman versions

### Podman

```nix
# modules/podman.nix
{ pkgs, ... }: {
  virtualisation.podman = {
    enable = true;
    dockerCompat = true;
  };

  # Create flashpods group (shared between flashpods user and spire-agent)
  users.groups.flashpods = {};
  
  # Create flashpods user for running containers
  users.users.flashpods = {
    isNormalUser = true;
    group = "flashpods";
  };

  # Podman socket for SPIRE agent
  # Owned by flashpods:flashpods, group-readable for SPIRE agent
  systemd.services.podman-socket-flashpods = {
    description = "Podman API socket for Flashpods";
    after = [ "network.target" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "simple";
      User = "flashpods";
      Group = "flashpods";
      ExecStart = "${pkgs.podman}/bin/podman system service --time=0 unix:///run/flashpods/podman.sock";
      Restart = "always";
    };
  };

  # Ensure socket directory exists with correct permissions
  systemd.tmpfiles.rules = [
    "d /run/flashpods 0750 flashpods flashpods -"
  ];

  # Pre-pull common images (run as flashpods user)
  system.activationScripts.pullImages = ''
    ${pkgs.podman}/bin/podman pull rust:latest || true
    ${pkgs.podman}/bin/podman pull rust:1.75 || true
    ${pkgs.podman}/bin/podman pull node:20 || true
    ${pkgs.podman}/bin/podman pull node:18 || true
    ${pkgs.podman}/bin/podman pull python:3.12 || true
    ${pkgs.podman}/bin/podman pull python:3.11 || true
    ${pkgs.podman}/bin/podman pull ubuntu:24.04 || true
    ${pkgs.podman}/bin/podman pull ubuntu:22.04 || true
    ${pkgs.podman}/bin/podman pull alpine:latest || true
  '';
}
```

### WireGuard

```nix
# modules/wireguard.nix
{ ... }: {
  networking.wireguard.interfaces.wg0 = {
    ips = [ "10.0.0.1/24" ];
    listenPort = 51820;
    privateKeyFile = "/etc/wireguard/private.key";
    
    peers = [{
      publicKey = "YOUR_LAPTOP_PUBLIC_KEY";
      allowedIPs = [ "10.0.0.2/32" ];
    }];
  };
}
```

### rsync Daemon

```nix
# modules/rsyncd.nix
{ ... }: {
  services.rsyncd = {
    enable = true;
    settings = {
      global = {
        address = "10.0.0.1";
        "hosts allow" = "10.0.0.2";  # Laptop only, not whole subnet
        "hosts deny" = "*";
      };
      uploads = {
        path = "/tmp/flashpods/uploads";
        "read only" = false;
        "use chroot" = true;
        "max connections" = 10;
      };
    };
  };

  systemd.tmpfiles.rules = [
    "d /tmp/flashpods/uploads 0755 flashpods flashpods -"
    "d /var/lib/flashpods/artifacts 0755 flashpods flashpods -"
    "d /var/lib/flashpods/db 0755 flashpods flashpods -"
    "d /var/log/flashpods 0755 flashpods flashpods -"
    # Note: /run/flashpods created by podman.nix with 0750 for socket security
  ];
}
```

### SPIRE

```nix
# modules/spire.nix
{ pkgs, ... }: {
  # SPIRE Server (runs as root, manages CA)
  systemd.services.spire-server = {
    description = "SPIRE Server";
    after = [ "network.target" ];
    wantedBy = [ "multi-user.target" ];
    
    serviceConfig = {
      Type = "simple";
      ExecStart = "${pkgs.spire}/bin/spire-server run -config /etc/spire/server.conf";
      Restart = "always";
    };
  };

  # SPIRE Agent (runs as root to read /proc cgroups, but in flashpods group for socket access)
  systemd.services.spire-agent = {
    description = "SPIRE Agent";
    after = [ "network.target" "spire-server.service" "podman-socket-flashpods.service" ];
    wantedBy = [ "multi-user.target" ];
    
    serviceConfig = {
      Type = "simple";
      # Run as root (needed to read /proc/<pid>/cgroup for any process)
      # But add to flashpods group for Podman socket access
      ExecStart = "${pkgs.spire}/bin/spire-agent run -config /etc/spire/agent.conf";
      Restart = "always";
      SupplementaryGroups = [ "flashpods" ];
    };
  };

  # SPIRE directories
  systemd.tmpfiles.rules = [
    "d /etc/spire 0755 root root -"
    "d /var/lib/spire 0700 root root -"
    "d /run/spire/sockets 0755 root root -"
  ];
}
```

### SPIRE/Podman Socket Permissions

**Ownership model:**

| Path | Owner | Group | Mode | Purpose |
|------|-------|-------|------|---------|
| /run/flashpods/ | flashpods | flashpods | 0750 | Socket directory |
| /run/flashpods/podman.sock | flashpods | flashpods | 0660 | Podman API socket |
| /run/flashpods/token.sock | flashpods | flashpods | 0660 | Token service socket |
| /run/spire/sockets/agent.sock | root | root | 0777 | SPIRE Workload API |

**How SPIRE Agent accesses Podman socket:**

1. Podman socket owned by `flashpods:flashpods` with mode `0660`
2. SPIRE Agent runs as root with `SupplementaryGroups = [ "flashpods" ]`
3. Group membership grants SPIRE Agent read access to Podman socket

**Service startup ordering:**

```
podman-socket-flashpods.service
         │
         ▼
   spire-agent.service (depends on podman socket)
         │
         ▼
   flashpods-api.service
         │
         ▼
   token-service.service
```

**Why SPIRE Agent runs as root:**
- Must read `/proc/<pid>/cgroup` for any container process
- Only root can read cgroup info for processes owned by other users
- Adding to `flashpods` group allows Podman socket access without running Podman as root

---

## Validation Tests

### SPIRE Validation Test

Deployment is considered valid only if:

```bash
podman run --rm \
  --label flashpods-job-type=worker \
  -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
  alpine \
  spire-agent api fetch x509 -socketPath /run/spire/sockets/agent.sock
```

Returns SVID with:
```
spiffe://flashpods.local/worker
```

### Token Exchange Validation Test

```bash
podman run --rm \
  --label flashpods-job-type=agent \
  -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
  -v /run/flashpods/token.sock:/run/flashpods/token.sock:ro \
  alpine sh -c '
    JWT=$(spire-agent api fetch jwt -audience flashpods-token-service -socketPath /run/spire/sockets/agent.sock | jq -r ".[].svids[0].svid")
    curl -s --unix-socket /run/flashpods/token.sock -H "Authorization: Bearer $JWT" http://localhost/tokens
  '
```

Returns:
```json
{
  "ANTHROPIC_API_KEY": "sk-...",
  "GITHUB_TOKEN": "ghp_..."
}
```

### Firewall Validation Test

From inside a container:

```bash
# Should fail (connection refused or timeout)
curl -v http://10.0.0.1:8080/health

# Should fail
curl -v rsync://10.0.0.1/uploads/

# Should succeed
curl -v https://api.anthropic.com/
```

---

## Implementation Phases

### Phase 1: Foundation (Week 1-2)
- [ ] Provision Hetzner server
- [ ] Install NixOS
- [ ] Configure WireGuard
- [ ] Setup Podman (rootless, flashpods user)
- [ ] Configure rsync daemon
- [ ] Configure firewall (block containers from API)
- [ ] Test connectivity

### Phase 2: API Core (Week 3-4)
- [ ] Scaffold Rust project (Axum + SQLite)
- [ ] Implement bearer token auth
- [ ] Implement job CRUD
- [ ] Implement container spawning (with labels)
- [ ] Implement log capture
- [ ] Implement timeout enforcement
- [ ] Unit + integration tests

### Phase 3: MCP Integration (Week 5-6)
- [ ] Create MCP server (TypeScript)
- [ ] Store API token securely
- [ ] Implement rsync file upload
- [ ] Implement all MCP tools
- [ ] Configure Claude Desktop
- [ ] End-to-end test

### Phase 4: Sub-Agents (Week 7-8)
- [ ] Build sub-agent container image
- [ ] Implement agent entrypoint script
- [ ] Task/context passing
- [ ] Git integration
- [ ] Test sub-agent workflow

### Phase 5: SPIRE + Token Exchange (Week 9-10)
- [ ] Derive NixOS cgroup path
- [ ] Set cgroup_prefix + container_index
- [ ] Write spire-server.conf and spire-agent.conf
- [ ] Run SPIRE validation test
- [ ] Build token exchange service (unix socket)
- [ ] Integrate with containers
- [ ] Run token exchange validation test

### Phase 6: Polish (Week 11-12)
- [ ] Error handling improvements
- [ ] Artifact cleanup automation
- [ ] Documentation
- [ ] Full E2E tests
- [ ] Run all validation tests
- [ ] Monitoring basics

---

## Future Features

Not in MVP, add later if needed:

| Feature | When to Add |
|---------|-------------|
| Job priority/queuing | When running 10+ concurrent jobs |
| Host restart recovery | When uptime matters |
| Per-user quotas | When adding users |
| Network egress filtering | When running untrusted code |
| eBPF observability | Learning project, post-MVP |
| WebSocket log streaming | If polling isn't responsive enough |
| Job dependencies | If workflows get complex |
| Custom image building | If pre-built images aren't enough |

---

## Quick Reference

### MCP Tools

```
spawn_worker(command, files?, image?, cpus?, memory_gb?, timeout_minutes?)
spawn_sub_agent(task, git_branch, files?, context?, timeout_minutes?)
get_job_status(job_id)
get_job_output(job_id, tail?)
get_job_artifacts(job_id)
download_artifact(job_id, artifact_name, save_to?)
list_jobs(status?, limit?)
kill_job(job_id)
```

### API Endpoints

```
POST   /uploads/{id}/finalize    Finalize upload (auth required)
GET    /uploads/{id}             Get upload status (auth required)
DELETE /uploads/{id}             Cancel upload (auth required)

POST   /jobs                     Create job (auth required)
GET    /jobs                     List jobs (auth required)
GET    /jobs/:id                 Get job details (auth required)
GET    /jobs/:id/output          Get logs (auth required)
GET    /jobs/:id/artifacts       List artifacts (auth required)
GET    /jobs/:id/artifacts/:name Download artifact (auth required)
DELETE /jobs/:id                 Kill job (auth required)

GET    /health                   Health check (no auth)
```

### File Locations (Host)

```
/tmp/flashpods/uploads/              Temporary file uploads
/var/lib/flashpods/artifacts/        Job artifacts
/var/lib/flashpods/db/               SQLite database
/var/log/flashpods/                  Job logs
/etc/flashpods/secrets.json          API keys (root only)
/etc/flashpods/api-token             API bearer token (root only)
/run/flashpods/token.sock            Token service socket
/run/flashpods/podman.sock           Podman socket for SPIRE
/run/spire/sockets/agent.sock        SPIRE workload API
```

### Container Mounts

| Path | Purpose | Worker | Sub-Agent |
|------|---------|--------|-----------|
| /work | Uploaded files | ro | rw |
| /artifacts | Output directory | rw | rw |
| /run/spire/sockets/agent.sock | SPIRE Workload API | ro | ro |
| /run/flashpods/token.sock | Token exchange | ro | ro |

### Default Resource Limits

| Resource | Worker | Sub-Agent |
|----------|--------|-----------|
| Default CPUs | 2 | 2 |
| Max CPUs | 8 | 4 |
| Default Memory | 4 GB | 4 GB |
| Max Memory | 16 GB | 8 GB |
| Default Timeout | 30 min | 60 min |
| Max Timeout | 120 min | 120 min |

---

*Your AI agent just got a build farm and a team. Flash in, flash out. ⚡*