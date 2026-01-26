# SPIRE Identity

## Purpose

SPIRE provides cryptographic workload identity for containers. Identity is derived from kernel cgroup metadata, not from anything the container claims.

## Workload Attestation Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SPIRE Attestation                                  │
│                                                                              │
│  1. Container connects to SPIRE Workload API socket:                        │
│     /run/spire/sockets/agent.sock                                           │
│                                                                              │
│  2. SPIRE Agent:                                                            │
│     a. Receives request                                                     │
│     b. Obtains caller PID from socket (SO_PEERCRED)                        │
│     c. Reads /proc/<pid>/cgroup                                             │
│     d. Extracts container ID from cgroup path                               │
│     e. Queries Podman (docker-compatible API) for container metadata        │
│     f. Converts metadata to selectors                                       │
│                                                                              │
│  3. Selectors generated (example):                                          │
│     - docker:label:flashpods-job-type:agent                                 │
│     - docker:label:flashpods-job-type:worker                                │
│                                                                              │
│  4. Selectors matched against registration entries:                         │
│     - spiffe://flashpods.local/agent                                        │
│     - spiffe://flashpods.local/worker                                       │
│                                                                              │
│  5. If matched, SPIRE issues SVID (JWT or X.509) to the container          │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Critical Point

> **Containers never claim identity.**
>
> Identity is derived from kernel cgroup metadata + runtime labels only.
> This is unforgeable from inside the container.

## SPIFFE IDs

| SPIFFE ID | Job Type | Description |
|-----------|----------|-------------|
| `spiffe://flashpods.local/agent` | agent | Claude sub-agent containers |
| `spiffe://flashpods.local/worker` | worker | Build/test worker containers |

## SPIRE Registration

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

## Container Labels

When creating containers, Flashpods API sets:

```bash
--label flashpods-job=true
--label flashpods-job-type=worker    # or "agent"
--label flashpods-job-id=job_xyz789
```

These labels are used by SPIRE for attestation and by Flashpods for reconciliation.

## Cgroup Configuration Derivation

SPIRE Agent needs to parse cgroup paths to extract container IDs. This is host-specific.

**Step 1: Find a container's cgroup path**

```bash
# Start a test container
podman run -d --name test-cgroup alpine sleep 3600

# Get its PID
PID=$(podman inspect test-cgroup --format '{{.State.Pid}}')

# Read cgroup
cat /proc/$PID/cgroup
```

**Example output (cgroupv1):**
```
12:freezer:/user.slice/user-1000.slice/user@1000.service/user.slice/libpod-abc123def456.scope
11:memory:/user.slice/user-1000.slice/user@1000.service/user.slice/libpod-abc123def456.scope
...
0::/user.slice/user-1000.slice/user@1000.service/user.slice/libpod-abc123def456.scope
```

**Example output (cgroupv2):**
```
0::/user.slice/user-1000.slice/user@1000.service/user.slice/libpod-abc123def456.scope
```

**Step 2: Derive configuration values**

From the cgroup path `libpod-abc123def456.scope`:
- Container ID is: `abc123def456`
- The prefix before the ID is: `libpod-`
- The suffix after the ID is: `.scope`

**SPIRE agent.conf docker attestor configuration:**

```hcl
WorkloadAttestor "docker" {
    plugin_data {
        # Path to Podman's docker-compatible socket
        docker_socket_path = "/run/flashpods/podman.sock"

        # cgroupv2 configuration
        container_id_cgroup_matchers = [
            # Match "libpod-<container_id>.scope" pattern
            "/libpod-([a-f0-9]+)\\.scope$"
        ]
    }
}
```

**For cgroupv1 systems, additional configuration may be needed:**

```hcl
WorkloadAttestor "docker" {
    plugin_data {
        docker_socket_path = "/run/flashpods/podman.sock"

        # cgroupv1 matchers
        cgroup_prefix = "/user.slice/user-1000.slice/user@1000.service/user.slice"
        cgroup_container_index = -1  # Last segment
    }
}
```

**Step 3: Validate configuration**

```bash
# Clean up test container
podman rm -f test-cgroup

# Run validation test (see Validation spec)
podman run --rm \
  --label flashpods-job-type=worker \
  -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
  alpine \
  spire-agent api fetch x509 -socketPath /run/spire/sockets/agent.sock
```

If attestation fails, check:
1. SPIRE agent logs: `journalctl -u spire-agent`
2. Podman socket permissions
3. Cgroup path regex matching

## Socket Permissions

| Path | Owner | Group | Mode | Purpose |
|------|-------|-------|------|---------|
| /run/spire/sockets/agent.sock | root | root | 0777 | SPIRE Workload API (world-accessible) |
| /run/flashpods/podman.sock | flashpods | flashpods | 0660 | Podman API (SPIRE agent needs access) |

**SPIRE Agent runs as root** with `SupplementaryGroups = [ "flashpods" ]`:
- Root needed to read `/proc/<pid>/cgroup` for any process
- flashpods group membership grants Podman socket access

## SPIRE Server Configuration

```hcl
# /etc/spire/server.conf
server {
    bind_address = "127.0.0.1"
    bind_port = "8081"
    trust_domain = "flashpods.local"
    data_dir = "/var/lib/spire/server"
    log_level = "INFO"

    ca_ttl = "24h"
    default_x509_svid_ttl = "1h"
    default_jwt_svid_ttl = "5m"
}

plugins {
    DataStore "sql" {
        plugin_data {
            database_type = "sqlite3"
            connection_string = "/var/lib/spire/server/datastore.sqlite3"
        }
    }

    NodeAttestor "join_token" {
        plugin_data {}
    }

    KeyManager "disk" {
        plugin_data {
            keys_path = "/var/lib/spire/server/keys.json"
        }
    }
}
```

## SPIRE Agent Configuration

```hcl
# /etc/spire/agent.conf
agent {
    data_dir = "/var/lib/spire/agent"
    log_level = "INFO"
    server_address = "127.0.0.1"
    server_port = "8081"
    socket_path = "/run/spire/sockets/agent.sock"
    trust_domain = "flashpods.local"
}

plugins {
    NodeAttestor "join_token" {
        plugin_data {}
    }

    KeyManager "disk" {
        plugin_data {
            directory = "/var/lib/spire/agent"
        }
    }

    WorkloadAttestor "docker" {
        plugin_data {
            docker_socket_path = "/run/flashpods/podman.sock"
            container_id_cgroup_matchers = [
                "/libpod-([a-f0-9]+)\\.scope$"
            ]
        }
    }
}
```

## Attestation Timeout

If SPIRE agent socket is unresponsive:
- Container entrypoint should timeout after 10 seconds
- Job fails with error: `spire_attestation_timeout`
- No retry (MCP can retry the job)

## Troubleshooting

| Symptom | Likely Cause | Fix |
|---------|--------------|-----|
| "no identity found" | Label mismatch | Check container has `flashpods-job-type` label |
| "workload not attested" | Cgroup parsing failed | Verify container_id_cgroup_matchers regex |
| "connection refused" | Socket not ready | Check SPIRE agent is running |
| "permission denied" | Socket permissions | Check /run/spire/sockets/agent.sock is 0777 |
| "docker attestor failed" | Podman socket | Check SPIRE agent can access Podman socket |

## Related Specs

- [Token Service](./09-token-service.md) - Uses SPIRE for authentication
- [Trust Boundary](./02-trust-boundary.md) - Security model
- [Container Mounts](./15-container-mounts.md) - Socket mounts
- [NixOS Configuration](./13-nixos.md) - SPIRE setup
- [Validation Tests](./14-validation.md) - Full test suite
