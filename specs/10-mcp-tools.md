# MCP Tools

## spawn_worker

Spawn a container to run a command (build, test, script). Returns immediately with job ID.

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
        default: "ubuntu:22.04",
        description: "Container image to use"
      },
      cpus: {
        type: "integer",
        default: 2,
        minimum: 1,
        maximum: 8,
        description: "CPU cores (integer only, clamped to max)"
      },
      memory_gb: {
        type: "integer",
        default: 4,
        minimum: 1,
        maximum: 16,
        description: "Memory in GB (integer only, clamped to max)"
      },
      timeout_minutes: {
        type: "integer",
        default: 30,
        minimum: 1,
        maximum: 120,
        description: "Max execution time before kill"
      }
    },
    required: ["command"]
  }
}
```

## spawn_sub_agent

Spawn another Claude instance to work on a task autonomously.

**Note:** The API `type` value is `agent` (not "sub_agent").

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
      cpus: {
        type: "integer",
        default: 2,
        minimum: 1,
        maximum: 4,
        description: "CPU cores (max 4 for agents)"
      },
      memory_gb: {
        type: "integer",
        default: 4,
        minimum: 1,
        maximum: 8,
        description: "Memory in GB (max 8 for agents)"
      },
      timeout_minutes: {
        type: "integer",
        default: 60,
        minimum: 1,
        maximum: 120
      }
    },
    required: ["task", "git_branch"]
  }
}
```

## get_job_status

Check job status.

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

**Response includes:**
- `status`: pending, starting, running, completed, failed, timed_out, cancelled, cleaning, cleaned
- `exit_code`: Only present for terminal states (0=success, 137=killed, etc.)
- `elapsed_seconds`: Time since job started
- `error`: Error message if failed

## get_job_output

Get stdout/stderr from a job.

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
        minimum: 1,
        maximum: 10000,
        description: "Number of lines from end"
      }
    },
    required: ["job_id"]
  }
}
```

## get_job_artifacts

List artifacts from a completed job.

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

## download_artifact

Download an artifact to local filesystem.

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
        description: "Local path to save file (defaults to current directory)"
      }
    },
    required: ["job_id", "artifact_name"]
  }
}
```

## list_jobs

List jobs, optionally filtered by status.

```typescript
{
  name: "list_jobs",
  description: "List jobs, optionally filtered by status.",
  inputSchema: {
    type: "object",
    properties: {
      status: {
        type: "string",
        enum: ["all", "running", "completed", "failed"],
        default: "all"
      },
      limit: {
        type: "integer",
        default: 20,
        minimum: 1,
        maximum: 100
      }
    }
  }
}
```

## kill_job

Terminate a running job immediately.

```typescript
{
  name: "kill_job",
  description: "Terminate a running job. Sends SIGTERM, then SIGKILL after grace period.",
  inputSchema: {
    type: "object",
    properties: {
      job_id: { type: "string" }
    },
    required: ["job_id"]
  }
}
```

## Quick Reference

```
spawn_worker(command, files?, image?, cpus?, memory_gb?, timeout_minutes?)
spawn_sub_agent(task, git_branch, files?, context?, cpus?, memory_gb?, timeout_minutes?)
get_job_status(job_id)
get_job_output(job_id, tail?)
get_job_artifacts(job_id)
download_artifact(job_id, artifact_name, save_to?)
list_jobs(status?, limit?)
kill_job(job_id)
```

## Error Handling

All tools may return errors. Common error patterns:

| Error | Meaning | Action |
|-------|---------|--------|
| job_not_found | Job ID doesn't exist | Check job ID |
| insufficient_resources | Host at capacity | Retry with backoff |
| upload_failed | rsync or finalization failed | Check disk space, retry |
| logs_not_available | Job hasn't started yet | Wait and retry |
| artifacts_deleted | Job was cleaned up | Cannot recover |

See [Error Codes](./17-error-codes.md) for complete list.

## Related Specs

- [Jobs](./03-jobs.md) - Job lifecycle
- [API](./11-api.md) - HTTP endpoints these tools call
- [Uploads](./05-uploads.md) - How files are uploaded
- [MCP Server](./16-mcp-server.md) - MCP server implementation
- [Error Codes](./17-error-codes.md) - All error responses
