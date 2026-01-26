# Flashpods Specifications

This folder contains focused specification documents for the Flashpods system, optimized for AI agent consumption.

## Quick Start

**New to Flashpods?** Start with [00-overview.md](./00-overview.md)

**Building the API?** Read [03-jobs.md](./03-jobs.md) → [11-api.md](./11-api.md) → [12-database.md](./12-database.md) → [17-error-codes.md](./17-error-codes.md)

**Building the MCP server?** Read [10-mcp-tools.md](./10-mcp-tools.md) → [16-mcp-server.md](./16-mcp-server.md) → [05-uploads.md](./05-uploads.md)

**Setting up infrastructure?** Read [13-nixos.md](./13-nixos.md) → [19-deployment.md](./19-deployment.md) → [14-validation.md](./14-validation.md)

**Troubleshooting?** See [18-operations.md](./18-operations.md)

## Spec Index

| # | Spec | Description |
|---|------|-------------|
| 00 | [Overview](./00-overview.md) | What Flashpods is, core model, key decisions |
| 01 | [Architecture](./01-architecture.md) | System diagram, components, storage locations |
| 02 | [Trust Boundary](./02-trust-boundary.md) | Security model, firewall rules, invariants |
| 03 | [Jobs](./03-jobs.md) | Job lifecycle, states, idempotency, crash recovery, exit codes |
| 04 | [Resource Scheduling](./04-resource-scheduling.md) | Admission control, no overcommit, resource caps |
| 05 | [Uploads](./05-uploads.md) | rsync flow, upload states, finalization, state transitions |
| 06 | [Logs](./06-logs.md) | Log capture, truncation, cleanup, format |
| 07 | [Artifacts](./07-artifacts.md) | Artifact write rules, validation, security, symlinks |
| 08 | [SPIRE](./08-spire.md) | Workload attestation, SPIFFE IDs, cgroup configuration |
| 09 | [Token Service](./09-token-service.md) | JWT exchange, token policy, container entrypoints |
| 10 | [MCP Tools](./10-mcp-tools.md) | Tool definitions for agents |
| 11 | [API](./11-api.md) | HTTP endpoints, request/response formats, rate limiting |
| 12 | [Database](./12-database.md) | SQLite schema, common queries |
| 13 | [NixOS](./13-nixos.md) | Nix modules, service configuration, all modules defined |
| 14 | [Validation](./14-validation.md) | Tests to verify deployment |
| 15 | [Container Mounts](./15-container-mounts.md) | Mount paths, permissions, complete Podman commands |
| 16 | [MCP Server](./16-mcp-server.md) | MCP server implementation, retry logic, error handling |
| 17 | [Error Codes](./17-error-codes.md) | Complete error code reference |
| 18 | [Operations](./18-operations.md) | Monitoring, alerting, troubleshooting, backup |
| 19 | [Deployment](./19-deployment.md) | Step-by-step deployment checklist |

## By Implementation Phase

### Phase 1: Foundation
- [01-architecture.md](./01-architecture.md) - System layout
- [13-nixos.md](./13-nixos.md) - WireGuard, Podman, rsync setup
- [02-trust-boundary.md](./02-trust-boundary.md) - Firewall configuration
- [19-deployment.md](./19-deployment.md) - Infrastructure checklist

### Phase 2: API Core
- [03-jobs.md](./03-jobs.md) - Job lifecycle
- [04-resource-scheduling.md](./04-resource-scheduling.md) - Admission control
- [05-uploads.md](./05-uploads.md) - File upload handling
- [06-logs.md](./06-logs.md) - Log capture
- [07-artifacts.md](./07-artifacts.md) - Artifact handling
- [11-api.md](./11-api.md) - HTTP endpoints
- [12-database.md](./12-database.md) - Database schema
- [17-error-codes.md](./17-error-codes.md) - Error handling

### Phase 3: MCP Integration
- [10-mcp-tools.md](./10-mcp-tools.md) - Tool definitions
- [16-mcp-server.md](./16-mcp-server.md) - MCP server implementation

### Phase 4: Sub-Agents
- [15-container-mounts.md](./15-container-mounts.md) - Container configuration
- [03-jobs.md](./03-jobs.md) - Agent job type
- [09-token-service.md](./09-token-service.md) - Agent entrypoint

### Phase 5: SPIRE + Token Exchange
- [08-spire.md](./08-spire.md) - Identity attestation
- [09-token-service.md](./09-token-service.md) - Token exchange
- [14-validation.md](./14-validation.md) - Validation tests

### Phase 6: Operations
- [18-operations.md](./18-operations.md) - Monitoring and troubleshooting
- [19-deployment.md](./19-deployment.md) - Deployment verification

## Key Terminology

| Term | Definition |
|------|------------|
| Job | A unit of work (worker or agent) that runs in a container |
| Worker | Job type that runs a command (build, test) with read-only /work |
| Agent | Job type that runs Claude autonomously with read-write /work |
| Upload | Files transferred via rsync, referenced by jobs |
| Artifact | Output files written to /artifacts by jobs |
| SPIFFE ID | Cryptographic identity issued by SPIRE |
| JWT-SVID | Short-lived JWT proving workload identity |

## Cross-References

Each spec contains a "Related Specs" section linking to relevant documents. Use these to navigate between related concepts.

## Conventions

- **Job type values:** Use `worker` or `agent` (not "sub-agent")
- **Resource values:** Integers only (no fractional CPUs or memory)
- **Exit codes:** 0=success, 1-125=command error, 128+N=signal N, 137=OOM/SIGKILL
- **Timestamps:** ISO 8601 format (e.g., `2026-01-21T10:30:00Z`)
- **IDs:** UUID v4 format for `client_job_id`, custom format for `job_id` and `upload_id`

## For AI Agents

These specs are designed to be:

1. **Self-contained** - Each spec covers one domain completely
2. **Concrete** - Includes code snippets, schemas, and examples
3. **Navigable** - Cross-references point to related specs
4. **Actionable** - Implementation details, not just concepts
5. **Error-aware** - Comprehensive error handling documented

When implementing a feature:
1. Read the relevant spec fully before starting
2. Check the "Related Specs" section for additional context
3. Use [17-error-codes.md](./17-error-codes.md) for error handling
4. Reference [14-validation.md](./14-validation.md) for testing

## Version

These specs correspond to Flashpods PRD v2.4 (January 2026).
