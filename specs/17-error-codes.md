# Error Codes

All API errors follow this format:

```json
{
  "error": "error_code",
  "message": "Human-readable description",
  "details": { ... }
}
```

## Authentication Errors

| Status | Error Code | Description |
|--------|------------|-------------|
| 401 | missing_authorization | No Authorization header provided |
| 401 | invalid_token | Bearer token is invalid or expired |
| 401 | malformed_authorization | Authorization header format is wrong |

## Rate Limiting Errors

| Status | Error Code | Description |
|--------|------------|-------------|
| 429 | rate_limited | Too many requests, retry after delay |

**Response includes:**
```json
{
  "error": "rate_limited",
  "message": "Rate limit exceeded",
  "retry_after_seconds": 5
}
```

## Upload Errors

| Status | Error Code | Description | Retryable |
|--------|------------|-------------|-----------|
| 404 | upload_not_found | Upload ID doesn't exist | No |
| 409 | upload_already_finalized | Upload was already finalized | No |
| 409 | upload_already_consumed | Upload was consumed by a job | No |
| 410 | upload_expired | Upload TTL exceeded | No |
| 507 | insufficient_storage | Server disk full | No (wait) |

## Job Creation Errors

| Status | Error Code | Description | Retryable |
|--------|------------|-------------|-----------|
| 400 | invalid_job_type | Type must be "worker" or "agent" | No |
| 400 | missing_command | Worker job requires command field | No |
| 400 | missing_task | Agent job requires task field | No |
| 400 | missing_git_branch | Agent job requires git_branch field | No |
| 400 | invalid_client_job_id | client_job_id must be valid UUID v4 | No |
| 400 | invalid_image | Image name is invalid or not allowed | No |
| 400 | invalid_cpus | CPUs must be integer 1-8 (worker) or 1-4 (agent) | No |
| 400 | invalid_memory | Memory must be integer 1-16 (worker) or 1-8 (agent) | No |
| 400 | invalid_timeout | Timeout must be integer 1-120 | No |
| 404 | upload_not_found | files_id references non-existent upload | No |
| 409 | upload_not_finalized | files_id references non-finalized upload | No |
| 429 | insufficient_resources | Host at capacity, try later | Yes |

**Insufficient resources response:**
```json
{
  "error": "insufficient_resources",
  "message": "Not enough resources to start job",
  "requested": { "cpus": 4, "memory_gb": 8 },
  "available": { "cpus": 2, "memory_gb": 6 },
  "host_capacity": { "cpus": 8, "memory_gb": 16 },
  "running_jobs": 3
}
```

## Job Query Errors

| Status | Error Code | Description |
|--------|------------|-------------|
| 404 | job_not_found | Job ID doesn't exist |

## Job Kill Errors

| Status | Error Code | Description |
|--------|------------|-------------|
| 404 | job_not_found | Job ID doesn't exist |
| 409 | job_already_terminal | Job is already completed/failed/cancelled |

## Log Errors

| Status | Error Code | Description |
|--------|------------|-------------|
| 404 | job_not_found | Job ID doesn't exist |
| 404 | logs_not_available | Job is in pending state (no container yet) |
| 410 | logs_deleted | Job is cleaned, logs were deleted |

## Artifact Errors

| Status | Error Code | Description |
|--------|------------|-------------|
| 400 | invalid_artifact_name | Artifact name fails validation |
| 404 | job_not_found | Job ID doesn't exist |
| 404 | artifact_not_found | Artifact doesn't exist |
| 404 | artifacts_not_available | Job hasn't reached terminal state |
| 410 | artifacts_deleted | Job is cleaned, artifacts were deleted |
| 503 | copy_in_progress | Artifacts still being copied, retry later |

## Internal Errors

| Status | Error Code | Description | Retryable |
|--------|------------|-------------|-----------|
| 500 | internal_error | Unexpected server error | Maybe |
| 500 | database_error | Database operation failed | Maybe |
| 500 | container_error | Podman operation failed | Maybe |
| 503 | service_unavailable | Service temporarily unavailable | Yes |

## Job Status Error Fields

When a job fails, the `error` field contains additional context:

| Error Value | Meaning |
|-------------|---------|
| oom_killed | Container exceeded memory limit |
| container_lost_on_recovery | Container was lost during API restart |
| container_not_found_on_recovery | Container never started before API crash |
| artifact_copy_failed | Failed to copy artifacts from container |
| artifact_copy_disk_full | Disk full during artifact copy |
| artifact_copy_permission | Permission denied during artifact copy |
| artifact_copy_container_gone | Container removed before artifact copy |
| spire_attestation_timeout | SPIRE attestation timed out |
| token_exchange_failed | Failed to get tokens from token service |

## Error Handling Best Practices

### For MCP Server

```typescript
async function handleApiError(response: Response): Promise<never> {
  const error = await response.json();

  switch (error.error) {
    case 'insufficient_resources':
      // Retry with exponential backoff
      throw new RetryableError(error.error, error.message);

    case 'rate_limited':
      // Wait for retry_after_seconds
      await sleep(error.retry_after_seconds * 1000);
      throw new RetryableError(error.error, error.message);

    case 'upload_expired':
    case 'logs_deleted':
    case 'artifacts_deleted':
      // Cannot recover, report to user
      throw new PermanentError(error.error, error.message);

    default:
      throw new FlashpodsError(error.error, error.message, response.status);
  }
}
```

### For Agents

When receiving errors from MCP tools:

| Error | Recommended Action |
|-------|-------------------|
| insufficient_resources | Wait 30s, retry. After 3 failures, report to user |
| upload_disk_full | Report to user, cannot proceed |
| job_not_found | Check job ID, may have been cleaned |
| logs_deleted | Job was cleaned, cannot retrieve logs |
| artifacts_deleted | Job was cleaned, artifacts lost |
| copy_in_progress | Wait 5s, retry |

## Validation Rules Reference

### client_job_id
- Must be valid UUID v4 format: `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`
- Max 36 characters
- Case-insensitive comparison

### artifact_name
- Cannot be empty
- Max 255 characters
- Cannot contain `/`, `\`, `..`, or NUL bytes
- Cannot have leading/trailing whitespace

### image
- Must be valid container image reference
- Format: `[registry/]name[:tag]`
- Max 255 characters

### command
- Cannot be empty
- Max 10000 characters
- Executed via `/bin/sh -c`

### task / context
- Max 100000 characters each

## Related Specs

- [API](./11-api.md) - Endpoint specifications
- [Jobs](./03-jobs.md) - Job lifecycle and error states
- [MCP Server](./16-mcp-server.md) - Error handling implementation
