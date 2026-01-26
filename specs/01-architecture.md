# Flashpods Architecture

## System Diagram

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
│  └──────────────────────────────────────────────────────────────────────────┘│
│                                                                               │
│  ┌─────────────────────────────────────────────────────────────────────────┐ │
│  │                         SPIRE                                            │ │
│  │  Server: CA + Registry                                                   │ │
│  │  Agent: Workload attestation via kernel cgroups                         │ │
│  │  Socket: /run/spire/sockets/agent.sock                                  │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
│                                                                               │
│  ┌─────────────────────────────────────────────────────────────────────────┐ │
│  │                         Token Service                                    │ │
│  │  Socket: /run/flashpods/token.sock (unix only, no TCP)                  │ │
│  │  Auth: JWT-SVID verification                                            │ │
│  │  Maps: SPIFFE ID → API tokens                                           │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────────────────┘
```

## Components

### 1. Flashpods API (Rust)

**Tech Stack:**
- Rust 2021
- Axum (HTTP)
- SQLite (sqlx)
- Tokio (async)
- Podman CLI

**Responsibilities:**
- Job CRUD operations
- Container lifecycle management
- Log capture
- Artifact management
- Bearer token authentication

### 2. MCP Server (TypeScript)

**Location:** Runs on user's laptop

**Responsibilities:**
- Store `FLASHPODS_API_TOKEN` (never shared)
- Translate MCP tool calls to HTTP API calls
- Execute rsync for file uploads
- Stream artifact downloads to local filesystem

### 3. Token Exchange Service (Rust)

**Transport:** Unix socket only (`/run/flashpods/token.sock`)

**Purpose:** Validate SPIRE JWT-SVIDs, return API tokens based on SPIFFE ID.

### 4. SPIRE

**Purpose:** Cryptographic workload identity via kernel cgroup attestation.

**Components:**
- Server: CA + Registry
- Agent: Workload attestation

## Storage Locations

```
/tmp/flashpods/uploads/{upload_id}/     <- Temp, deleted after start
/var/lib/flashpods/artifacts/{job_id}/  <- TTL-only retention (1hr)
/var/lib/flashpods/db/flashpods.db      <- Job metadata
/var/log/flashpods/{job_id}.log         <- TTL-only retention (24hr)
```

## Related Specs

- [Trust Boundary](./02-trust-boundary.md) - Security model
- [Jobs](./03-jobs.md) - Job management
- [NixOS Configuration](./13-nixos.md) - Infrastructure setup
