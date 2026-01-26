# File Uploads

## Why rsync Daemon?

| Approach | SSH Keys | Delta Sync | Security |
|----------|----------|------------|----------|
| rsync + SSH | Needs keys | Yes | Static secret |
| rsync daemon | No keys | Yes | IP allowlist + firewall |
| HTTP upload | No keys | No | Simple |

**rsync daemon gives us delta sync without SSH keys.**

## Upload States

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
| `consumed` | Job reached `running` state, upload directory deleted |
| `expired` | TTL exceeded, marked for deletion |

## State Transition Rules

| From | To | Trigger |
|------|----|---------|
| uploading | finalized | `POST /uploads/{id}/finalize` called |
| uploading | expired | TTL (30 min) exceeded without finalization |
| finalized | consumed | Job referencing this upload reaches `running` state |
| finalized | expired | TTL (60 min) exceeded without job consumption |
| finalized | expired | Job fails/cancelled before reaching `running` state |

**Critical timing:** Upload moves to `consumed` exactly when container state becomes `running` (not `starting`). This ensures the container has successfully started before we delete the upload directory.

## Upload Flow

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
│     + Bearer    │                      │     ↓           │
│     files_id:   │                      │  Validates:     │
│     upload_id   │                      │  - upload exists│
│                 │                      │  - state=final. │
│                 │                      │                 │
│  5. Container   │                      │  Job: starting  │
│     starting    │                      │  Upload: final. │
│                 │                      │                 │
│  6. Container   │                      │  Job: running   │
│     running     │                      │  Upload: consumed│
│                 │                      │  (dir deleted)  │
└─────────────────┘                      └─────────────────┘
```

## rsync Command (MCP Side)

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

## Finalization

Jobs can only reference uploads in state `finalized`. MCP must call:

```http
POST /uploads/{id}/finalize
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

**Error responses:**

| Status | Error Code | Condition |
|--------|------------|-----------|
| 404 | upload_not_found | Upload doesn't exist |
| 409 | upload_already_finalized | Already finalized |
| 409 | upload_already_consumed | Already consumed by a job |
| 410 | upload_expired | Upload TTL exceeded |
| 507 | insufficient_storage | Disk quota exceeded |

## Upload Quotas

| Constraint | Default | Configurable |
|------------|---------|--------------|
| Max upload size | 2 GB | Yes |
| Max total upload disk usage | 10 GB | Yes |
| Max upload TTL (non-finalized) | 30 minutes | Yes |
| Max upload TTL (finalized, unused) | 60 minutes | Yes |

**When quotas are exceeded:**
- rsync uploads fail with ENOSPC (disk full)
- API returns `507 Insufficient Storage`
- Partial uploads are cleaned up automatically

## Cleanup Rules

| Condition | Action |
|-----------|--------|
| Non-finalized upload exceeds TTL | Delete immediately |
| Finalized but unused upload exceeds TTL | Delete immediately |
| Upload consumed (job reached `running`) | Delete immediately after container start |
| Job fails during `starting` state | Delete upload immediately |
| Job cancelled before `running` state | Delete upload immediately |
| rsync fails mid-upload (ENOSPC, network) | Partial directory cleaned by cleanup daemon |

## Integrity Invariant

> **Container sees an immutable snapshot of a finalized upload.**

Guarantees:
1. Upload directory is mounted **read-only** into container (workers) or **read-write** (agents)
2. Upload state must be `finalized` before job creation succeeds
3. Upload directory is deleted **only after** container reaches `running` state
4. No concurrent modification of upload directory once finalized
5. File permissions preserved from upload (owned by `flashpods` user after mount)

## MCP Complete Flow

```typescript
async function spawnWorker(command: string, files: FileSpec): Promise<string> {
  // 1. Generate upload ID
  const uploadId = `upload_${crypto.randomUUID().slice(0, 12)}`;

  // 2. rsync files (may fail with ENOSPC)
  try {
    await rsyncFiles(files.local_path, uploadId, files.exclude);
  } catch (e) {
    if (e.message.includes('No space left')) {
      throw new Error('Upload failed: server disk full');
    }
    throw e;
  }

  // 3. Finalize upload (required step)
  const finalizeResp = await fetch(`http://10.0.0.1:8080/uploads/${uploadId}/finalize`, {
    method: 'POST',
    headers: { 'Authorization': `Bearer ${API_TOKEN}` }
  });

  if (!finalizeResp.ok) {
    const error = await finalizeResp.json();
    throw new Error(`Finalize failed: ${error.message}`);
  }

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
    })
  });

  if (!response.ok) {
    const error = await response.json();
    throw new Error(`Job creation failed: ${error.message}`);
  }

  return (await response.json()).job_id;
}
```

## Cleanup Daemon

```rust
// Runs every minute
async fn cleanup_expired_uploads() {
    let now = Utc::now();

    // 1. Delete expired uploads (uploading or finalized but not consumed)
    let expired = sqlx::query!(
        "SELECT id, state FROM uploads WHERE expires_at < ? AND state IN ('uploading', 'finalized')",
        now
    ).fetch_all(&pool).await?;

    for upload in expired {
        // Delete from filesystem (ignore if already gone)
        let path = format!("/tmp/flashpods/uploads/{}", upload.id);
        if let Err(e) = fs::remove_dir_all(&path) {
            if e.kind() != io::ErrorKind::NotFound {
                warn!("Failed to delete upload {}: {}", upload.id, e);
                continue; // Retry next tick
            }
        }

        // Update state
        sqlx::query!(
            "UPDATE uploads SET state = 'expired' WHERE id = ?",
            upload.id
        ).execute(&pool).await?;

        info!("Expired upload {} (was {})", upload.id, upload.state);
    }

    // 2. Clean up orphaned directories (exist on disk but not in DB)
    let upload_dir = Path::new("/tmp/flashpods/uploads");
    if let Ok(entries) = fs::read_dir(upload_dir) {
        for entry in entries.flatten() {
            let dir_name = entry.file_name().to_string_lossy().to_string();
            if !db.upload_exists(&dir_name).await {
                warn!("Removing orphaned upload directory: {}", dir_name);
                fs::remove_dir_all(entry.path()).ok();
            }
        }
    }
}
```

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | /uploads/{id}/finalize | Mark upload as finalized |
| GET | /uploads/{id} | Get upload status |
| DELETE | /uploads/{id} | Cancel/delete upload |

See [API Specification](./11-api.md) for full request/response details.
See [Error Codes](./17-error-codes.md) for all error responses.

## Related Specs

- [Jobs](./03-jobs.md) - Job creation references uploads
- [API](./11-api.md) - Full endpoint details
- [Database Schema](./12-database.md) - Upload tables
- [Error Codes](./17-error-codes.md) - Error responses
