# Artifacts

## Artifact Write Rules

Jobs write outputs **only** to `/artifacts` inside the container.

```bash
# Inside container
cp target/release/myapp /artifacts/
echo "Build complete" > /artifacts/summary.txt
```

## /artifacts Directory Setup

The `/artifacts` directory is created by Flashpods with these properties:

| Property | Value |
|----------|-------|
| Permissions | 0755 (drwxr-xr-x) |
| Owner | Container user (flashpods via userns) |
| Writable | Yes |
| Subdirectories | Allowed |
| Symlinks | Allowed (with validation) |

**Subdirectories:** Jobs can create nested directories under `/artifacts`. The entire tree is copied.

**Symlinks:** Symlinks are allowed but must resolve within `/artifacts`. Symlinks pointing outside are rejected during copy.

## Server-Side Storage

After container exits, artifacts are copied to:

```
/var/lib/flashpods/artifacts/{job_id}/
```

**Copy timing:** Artifacts are copied when container reaches terminal state (`completed`, `failed`, `timed_out`, `cancelled`). Copy happens before job status is updated.

**Copy mechanism:**
```bash
podman cp {container_id}:/artifacts/. /var/lib/flashpods/artifacts/{job_id}/
```

## Artifact Constraints

| Constraint | Default | Configurable |
|------------|---------|--------------|
| Max artifact size per file | 1 GB | Yes |
| Max total artifacts per job | 2 GB | Yes |
| Max number of artifacts per job | 200 | Yes |
| Max filename length | 255 chars | No |
| Artifact retention TTL | 1 hour | Yes |

## Overwrite Semantics

**Artifact names must be unique within a job.**

If a file is written multiple times:
- Last write wins (normal filesystem behavior inside container)
- No conflict error during execution

If copy to server would overwrite:
- Copy fails with error
- Job marked `failed` with error: `artifact_copy_conflict`

This prevents unexpected overwrites from race conditions during copy.

## Artifact Name Safety (Mandatory)

Artifact names are treated as **filenames, not paths**.

The server **MUST reject** artifact names containing:
- `/` or `\` (path separators)
- `..` (parent directory traversal)
- NUL bytes (`\0`)
- Leading or trailing whitespace
- Empty string
- Names starting with `.` (hidden files) - allowed but logged

**Valid:** `myapp`, `build.log`, `report-2026-01-21.pdf`, `.hidden` (allowed with warning)

**Invalid:** `../../../etc/passwd`, `foo/bar.txt`, ``, `  `

## Symlink Validation

Symlinks in `/artifacts` are allowed but validated during copy:

```rust
fn validate_symlink(artifact_dir: &Path, link_path: &Path) -> Result<(), Error> {
    let target = fs::read_link(link_path)?;
    let resolved = link_path.parent().unwrap().join(&target).canonicalize()?;

    if !resolved.starts_with(artifact_dir) {
        return Err(Error::SymlinkEscape {
            link: link_path.to_path_buf(),
            target: resolved,
        });
    }
    Ok(())
}
```

Symlinks pointing outside `/artifacts` cause:
- Copy to fail for that artifact
- Warning logged
- Other artifacts still copied
- Job completes (not failed)

## Artifact Validation

```rust
fn validate_artifact_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::InvalidArtifactName("empty name"));
    }
    if name.len() > 255 {
        return Err(Error::InvalidArtifactName("name too long (max 255)"));
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
    if name.starts_with('.') {
        warn!("Hidden artifact: {}", name); // Allowed but logged
    }
    Ok(())
}
```

## Path Traversal Prevention

Artifact downloads **MUST resolve strictly within:**

```
/var/lib/flashpods/artifacts/{job_id}/
```

```rust
fn safe_artifact_path(job_id: &str, name: &str) -> Result<PathBuf, Error> {
    validate_artifact_name(name)?;

    let base = PathBuf::from("/var/lib/flashpods/artifacts").join(job_id);
    let full_path = base.join(name);

    // Canonicalize and verify still under base
    let canonical = full_path.canonicalize()?;
    if !canonical.starts_with(&base) {
        return Err(Error::PathTraversal);
    }

    Ok(canonical)
}
```

## Cleanup Semantics

- Artifacts are deleted after TTL, **regardless of download status**
- Partial downloads do not affect cleanup
- Jobs are marked `cleaned` once logs and artifacts successfully deleted

**No race conditions. No hidden state. No "did the client finish downloading?".**

## Artifact Lifecycle

```
┌──────────┐     ┌───────────┐     ┌────────────┐
│ created  │ ──► │ available │ ──► │  deleted   │
└──────────┘     └───────────┘     └────────────┘
```

| State | Description |
|-------|-------------|
| `created` | Container wrote file to /artifacts |
| `available` | Job in terminal state, artifact copied and accessible via API |
| `deleted` | TTL expired, job entering `cleaned` state |

## Concurrent Access

- **During execution:** Container writes, no API access
- **After completion:** API serves, container gone
- **During copy:** Race window exists; API waits for copy to complete

Artifact listing returns point-in-time snapshot. If copy is in progress, listing may return partial results with `copy_in_progress: true`.

## Copy Failure Handling

| Failure | Action |
|---------|--------|
| Disk full (ENOSPC) | Job marked `failed`, error: `artifact_copy_disk_full` |
| Permission denied | Job marked `failed`, error: `artifact_copy_permission` |
| Symlink escape | Artifact skipped, warning logged, job completes |
| Container already removed | Job marked `failed`, error: `artifact_copy_container_gone` |
| Partial copy (crash) | Cleanup daemon removes partial artifacts on next run |

## API Endpoints

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
  "expires_at": "2026-01-21T11:35:00Z",
  "copy_in_progress": false
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

**Error responses:**

| Status | Error Code | Condition |
|--------|------------|-----------|
| 400 | invalid_artifact_name | Name fails validation |
| 404 | job_not_found | Job doesn't exist |
| 404 | artifact_not_found | Artifact doesn't exist |
| 410 | artifacts_deleted | Job is `cleaned` |
| 503 | copy_in_progress | Artifacts still being copied (retry later) |

## Related Specs

- [Jobs](./03-jobs.md) - Job lifecycle
- [Container Mounts](./15-container-mounts.md) - /artifacts mount
- [API](./11-api.md) - Full endpoint details
- [Database Schema](./12-database.md) - Artifacts table
- [Error Codes](./17-error-codes.md) - All error responses
