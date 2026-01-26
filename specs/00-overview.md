# Flashpods Overview

## What Is Flashpods

Flashpods is a compute backend that gives AI coding agents the ability to:

1. **Spawn workers** for long-running tasks (builds, tests, scripts)
2. **Spawn sub-agents** to work on tasks in parallel
3. **Execute code** in isolated, ephemeral containers
4. **Offload resource-heavy work** without blocking the main agent

## Core Model

**Ephemeral containers:**
- Files go IN at job start (uploaded for that job)
- Files come OUT via /artifacts (explicitly retrieved)
- Container dies, everything gone

**No persistent workspaces. No shared state between jobs.**

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Execution model | Ephemeral containers | Simple, secure, no state leakage |
| File upload | rsync daemon (no SSH) | Delta sync efficiency, no static keys |
| API auth | Bearer token (MCP only) | Defense in depth beyond WireGuard |
| Container runtime | Podman rootless | Security without root |
| Identity | SPIRE (kernel cgroups) | Cryptographic, unforgeable identity |
| Secret delivery | Token exchange via unix socket | Containers can't intercept |
| Sub-agent recursion | Impossible (firewall) | System-enforced, not convention |
| Job retry | Manual (agent decides) | Keep it simple |
| Log access | Polling | Simpler than WebSocket |
| Podman user | `flashpods` with `--userns=keep-id` | Stable cgroup layout, socket permissions |

## Usage Example

```
Agent: spawn_worker({
         command: "cargo build --release",
         files: { local_path: "/Users/me/myapp" }
       })
     <- job_id: "job_abc123"

Agent: *continues other work*

Agent: get_job_status("job_abc123")
     <- { status: "completed", exit_code: 0, duration: "8m 32s" }

Agent: get_job_artifacts("job_abc123")
     <- [ "myapp (4.2 MB)", "build.log" ]
```

## Related Specs

- [Architecture](./01-architecture.md) - System components and layout
- [Jobs](./03-jobs.md) - Job lifecycle and semantics
- [MCP Tools](./10-mcp-tools.md) - Tool definitions for agents
