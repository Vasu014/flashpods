# API Specification

## Authentication

All requests (except `/health`) require:
```
Authorization: Bearer <FLASHPODS_API_TOKEN>
```

Requests without valid token receive `401 Unauthorized`.

## Rate Limiting

| Limit | Value |
|-------|-------|
| Requests per second | 100 |
| Burst allowance | 10 |
| Exceeded response | 429 Too Many Requests |

## Endpoints Summary

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

---

## Upload Endpoints

### POST /uploads/{id}/finalize

Mark upload as finalized (ready for job creation).

**Request:**
```http
POST /uploads/upload_abc123/finalize HTTP/1.1
Authorization: Bearer <token>
```

**Response (200):**
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

**Errors:** See [Error Codes](./17-error-codes.md#upload-errors)

### GET /uploads/{id}

Get upload status.

**Response (200):**
```json
{
  "upload_id": "upload_abc123",
  "state": "finalized",
  "size_bytes": 15728640,
  "file_count": 847,
  "created_at": "2026-01-21T10:25:00Z",
  "finalized_at": "2026-01-21T10:30:00Z",
  "expires_at": "2026-01-21T11:00:00Z"
}
```

### DELETE /uploads/{id}

Cancel/delete an upload.

**Response (204):** No content

---

## Job Endpoints

### POST /jobs

Create a new job.

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

**Request fields:**

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| client_job_id | string | No | - | Idempotency key (UUID v4) |
| type | string | Yes | - | "worker" or "agent" |
| command | string | worker only | - | Shell command to run |
| task | string | agent only | - | Task description |
| context | string | No | - | Additional context (agent) |
| git_branch | string | agent only | - | Git branch name |
| files_id | string | No | - | Upload ID (must be finalized) |
| image | string | No | ubuntu:22.04 | Container image |
| cpus | integer | No | 2 | CPU cores (1-8 worker, 1-4 agent) |
| memory_gb | integer | No | 4 | Memory GB (1-16 worker, 1-8 agent) |
| timeout_minutes | integer | No | 30/60 | Timeout (1-120) |

**Response (201 - new job created):**
```json
{
  "job_id": "job_xyz789",
  "status": "starting",
  "created": true
}
```

**Response (200 - existing job returned via idempotency):**
```json
{
  "job_id": "job_xyz789",
  "status": "running",
  "created": false,
  "message": "Existing job returned (idempotent)"
}
```

**Errors:** See [Error Codes](./17-error-codes.md#job-creation-errors)

### GET /jobs

List jobs.

**Query params:**
- `status` - Filter by status (optional): all, running, completed, failed
- `limit` - Max results (default: 20, max: 100)

**Response (200):**
```json
{
  "jobs": [
    {
      "id": "job_xyz789",
      "type": "worker",
      "status": "completed",
      "command": "cargo build --release",
      "exit_code": 0,
      "created_at": "2026-01-18T10:30:00Z",
      "completed_at": "2026-01-18T10:38:34Z"
    }
  ],
  "total": 1
}
```

### GET /jobs/:id

Get job details.

**Response (200) - running:**
```json
{
  "id": "job_xyz789",
  "type": "worker",
  "status": "running",
  "command": "cargo build --release",
  "image": "rust:latest",
  "cpus": 4,
  "memory_gb": 8,
  "timeout_minutes": 30,
  "created_at": "2026-01-18T10:30:00Z",
  "started_at": "2026-01-18T10:30:02Z",
  "elapsed_seconds": 145
}
```

**Response (200) - completed:**
```json
{
  "id": "job_xyz789",
  "type": "worker",
  "status": "completed",
  "command": "cargo build --release",
  "image": "rust:latest",
  "exit_code": 0,
  "created_at": "2026-01-18T10:30:00Z",
  "started_at": "2026-01-18T10:30:02Z",
  "completed_at": "2026-01-18T10:38:34Z",
  "duration_seconds": 512
}
```

**Response (200) - failed:**
```json
{
  "id": "job_xyz789",
  "type": "worker",
  "status": "failed",
  "command": "cargo build --release",
  "exit_code": 1,
  "error": "Build failed with 3 errors",
  "created_at": "2026-01-18T10:30:00Z",
  "started_at": "2026-01-18T10:30:02Z",
  "completed_at": "2026-01-18T10:31:15Z"
}
```

**Response (200) - timed_out:**
```json
{
  "id": "job_xyz789",
  "type": "worker",
  "status": "timed_out",
  "exit_code": 137,
  "timeout_minutes": 30,
  "actual_runtime_seconds": 1800,
  "error": "Job exceeded timeout limit"
}
```

### GET /jobs/:id/output

Get stdout/stderr.

**Query params:**
- `tail` - Number of lines from end (default: 100, max: 10000)

**Response (200):**
```json
{
  "output": "[10:30:00.123] Compiling myapp v0.1.0\n...",
  "lines": 100,
  "truncated": true,
  "total_bytes": 52428800
}
```

| Field | Description |
|-------|-------------|
| `output` | Log content (last N lines from captured log) |
| `lines` | Number of lines returned |
| `truncated` | True if log exceeded max size during capture |
| `total_bytes` | Total size of captured log file |

**Errors:** See [Error Codes](./17-error-codes.md#log-errors)

### GET /jobs/:id/artifacts

List artifacts.

**Response (200):**
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
  "expires_at": "2026-01-21T11:35:00Z",
  "copy_in_progress": false
}
```

### GET /jobs/:id/artifacts/:name

Download artifact as binary stream.

**Response (200):**
```http
HTTP/1.1 200 OK
Content-Type: application/octet-stream
Content-Length: 4404224
Content-Disposition: attachment; filename="myapp"

<binary data>
```

**Errors:** See [Error Codes](./17-error-codes.md#artifact-errors)

### DELETE /jobs/:id

Kill a running job.

**Response (200):**
```json
{
  "job_id": "job_xyz789",
  "status": "cancelled",
  "message": "Job termination initiated"
}
```

**Errors:**
- `404` if job not found
- `409` if job already in terminal state

---

## Health Endpoint

### GET /health

Health check (no authentication required).

**Response (200):**
```json
{
  "status": "healthy",
  "version": "0.1.0",
  "uptime_seconds": 3600
}
```

## Common Response Headers

All responses include:
```http
X-Request-Id: <uuid>
X-RateLimit-Limit: 100
X-RateLimit-Remaining: 95
X-RateLimit-Reset: 1706356800
```

## Related Specs

- [Jobs](./03-jobs.md) - Job lifecycle details
- [Uploads](./05-uploads.md) - Upload lifecycle
- [Artifacts](./07-artifacts.md) - Artifact handling
- [MCP Tools](./10-mcp-tools.md) - Tools that call these endpoints
- [Error Codes](./17-error-codes.md) - Complete error reference
