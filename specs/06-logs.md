# Logs

## Log Capture Model

Each job writes stdout/stderr to a server-side log file:

```
/var/log/flashpods/{job_id}.log
```

Logs are captured in real-time as the container runs.

## Capture Timing

| Event | Log Capture Status |
|-------|-------------------|
| Job created (pending) | No capture yet |
| Container starting | Capture begins (includes pull/init output) |
| Container running | Capture continues |
| Container exits | Capture ends |
| Job terminal state | Logs available for retrieval |
| Job cleaned | Logs deleted |

**Logs are available in any state >= `starting`**, including while the container is still running.

## Log Format

Logs are stored as plain text with line prefixes:

```
[10:30:00.123] Compiling myapp v0.1.0
[10:30:00.456] Downloading crates...
[10:30:01.789]    Compiling serde v1.0
```

**Format details:**
- Timestamp prefix: `[HH:MM:SS.mmm]` (24-hour, milliseconds)
- stdout and stderr are **interleaved** in order received
- No JSON wrapping in MVP
- Line-buffered capture (output appears as lines complete)

## Log Limits

| Constraint | Default | Configurable |
|------------|---------|--------------|
| Max log size per job | 50 MB | Yes |
| Max total log disk usage | 10 GB | Yes |
| Log retention TTL (completed jobs) | 24 hours | Yes |

## Truncation Semantics

If log exceeds max size:
1. Flashpods **stops capturing further output** (first 50 MB retained)
2. Job **continues running** (not killed)
3. Log is marked `truncated=true`
4. A truncation marker is appended to the log

**Important:** Truncation keeps the **beginning** of the log (first N bytes). The `tail` parameter on GET returns the **last N lines** of whatever was captured.

Example with 50 MB limit:
- Log reaches 50 MB at timestamp 10:35:00
- Flashpods stops capturing, appends: `[10:35:00.000] [TRUNCATED - log exceeded 50 MB limit]`
- Job continues running for 5 more minutes
- Final log contains: first 50 MB + truncation marker
- GET with `tail=100` returns last 100 lines of the 50 MB captured

## API Response

**GET /jobs/:id/output**

Query params: `?tail=100` (default: 100, max: 10000)

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
| `truncated` | True if log exceeded max size limit during capture |
| `total_bytes` | Total size of captured log file |

**Error responses:**

| Status | Error Code | Condition |
|--------|------------|-----------|
| 404 | job_not_found | Job doesn't exist |
| 404 | logs_not_available | Job in `pending` state (no container yet) |
| 410 | logs_deleted | Job is `cleaned`, logs were deleted |

## Log Cleanup

| Condition | Action |
|-----------|--------|
| Job completed + TTL exceeded | Delete log file |
| Job failed + TTL exceeded | Delete log file |
| Job timed_out + TTL exceeded | Delete log file |
| Job cancelled + TTL exceeded | Delete log file |
| Total disk usage exceeded | Delete oldest completed job logs first |

**Cleanup is TTL-based, not download-based.** Logs are deleted after TTL regardless of whether they were retrieved.

## Rotation

Log files are not rotated during job execution. Each job gets a single log file that grows until:
- Job completes/fails
- Truncation limit reached

System-level log rotation (logrotate) is not used for job logs. The cleanup daemon handles deletion based on TTL.

## Implementation Notes

```rust
// Log writer (runs during container execution)
async fn capture_logs(job_id: &str, container_id: &str) {
    let log_path = format!("/var/log/flashpods/{}.log", job_id);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    let mut total_bytes = 0u64;
    let max_bytes = 50 * 1024 * 1024; // 50 MB
    let mut truncated = false;

    let stream = podman_logs_stream(container_id).await?;

    while let Some(line) = stream.next().await {
        if truncated {
            continue; // Drain stream but don't write
        }

        let timestamp = Utc::now().format("[%H:%M:%S%.3f]");
        let log_line = format!("{} {}\n", timestamp, line);

        if total_bytes + log_line.len() as u64 > max_bytes {
            let truncation_msg = format!("{} [TRUNCATED - log exceeded {} MB limit]\n",
                timestamp, max_bytes / 1024 / 1024);
            file.write_all(truncation_msg.as_bytes())?;
            truncated = true;
            db.set_log_truncated(job_id, true).await;
            continue;
        }

        file.write_all(log_line.as_bytes())?;
        total_bytes += log_line.len() as u64;
    }
}
```

## Related Specs

- [Jobs](./03-jobs.md) - Job lifecycle
- [API](./11-api.md) - GET /jobs/:id/output endpoint
- [Error Codes](./17-error-codes.md) - Error responses
- [Operations](./18-operations.md) - Monitoring log disk usage
