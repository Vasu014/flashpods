# Deployment Checklist

## Prerequisites

- [ ] Hetzner server provisioned (8+ vCPU, 16GB+ RAM)
- [ ] NixOS installed
- [ ] SSH access configured
- [ ] Domain/IP noted for WireGuard

## Phase 1: Base Infrastructure

### WireGuard Setup

- [ ] Generate server keys: `wg genkey | tee privatekey | wg pubkey > publickey`
- [ ] Generate laptop keys on laptop
- [ ] Configure server in `modules/wireguard.nix`
- [ ] Configure laptop WireGuard client
- [ ] Test connectivity: `ping 10.0.0.1` from laptop

### User & Directory Setup

- [ ] Create flashpods user and group
- [ ] Create directories:
  ```
  /tmp/flashpods/uploads (0755 flashpods:flashpods)
  /var/lib/flashpods/artifacts (0755 flashpods:flashpods)
  /var/lib/flashpods/db (0755 flashpods:flashpods)
  /var/log/flashpods (0755 flashpods:flashpods)
  /run/flashpods (0750 flashpods:flashpods)
  /etc/flashpods (0700 root:root)
  ```

### Firewall Configuration

- [ ] Enable nftables
- [ ] Configure WireGuard port (51820) open to internet
- [ ] Configure API port (8080) restricted to laptop
- [ ] Configure rsync port (873) restricted to laptop
- [ ] Block container access to API and rsync
- [ ] Test firewall rules

### Podman Setup

- [ ] Enable rootless Podman for flashpods user
- [ ] Configure Podman socket at `/run/flashpods/podman.sock`
- [ ] Pre-pull common images:
  ```bash
  podman pull rust:latest
  podman pull node:20
  podman pull python:3.12
  podman pull ubuntu:22.04
  ```
- [ ] Verify Podman works: `podman run --rm alpine echo hello`

### rsync Daemon

- [ ] Configure rsyncd with uploads module
- [ ] Set hosts allow to laptop IP only
- [ ] Test from laptop: `rsync rsync://10.0.0.1/uploads/`

## Phase 2: Secrets & Identity

### Secrets Setup

- [ ] Create `/etc/flashpods/secrets.json`:
  ```json
  {
    "anthropic_api_key": "sk-ant-...",
    "github_token": "ghp_..."
  }
  ```
- [ ] Set permissions: `chmod 600 /etc/flashpods/secrets.json`

### API Token

- [ ] Generate token: `openssl rand -hex 32`
- [ ] Save to `/etc/flashpods/api-token`
- [ ] Set permissions: `chmod 600 /etc/flashpods/api-token`

### SPIRE Server

- [ ] Create `/etc/spire/server.conf`
- [ ] Start SPIRE server: `systemctl start spire-server`
- [ ] Create join token: `spire-server token generate -spiffeID spiffe://flashpods.local/host`

### SPIRE Agent

- [ ] Derive cgroup configuration from test container
- [ ] Create `/etc/spire/agent.conf` with correct matchers
- [ ] Start SPIRE agent: `systemctl start spire-agent`
- [ ] Verify agent is attested: `spire-server agent list`

### SPIRE Registration

- [ ] Register worker entry:
  ```bash
  spire-server entry create \
    -spiffeID spiffe://flashpods.local/worker \
    -parentID spiffe://flashpods.local/host \
    -selector docker:label:flashpods-job-type:worker
  ```
- [ ] Register agent entry:
  ```bash
  spire-server entry create \
    -spiffeID spiffe://flashpods.local/agent \
    -parentID spiffe://flashpods.local/host \
    -selector docker:label:flashpods-job-type:agent
  ```

## Phase 3: Services

### Token Service

- [ ] Deploy token service binary
- [ ] Create systemd service
- [ ] Start service: `systemctl start flashpods-token-service`
- [ ] Verify socket exists: `ls /run/flashpods/token.sock`

### Flashpods API

- [ ] Deploy API binary
- [ ] Initialize database:
  ```bash
  sqlite3 /var/lib/flashpods/db/flashpods.db < schema.sql
  ```
- [ ] Create systemd service
- [ ] Start service: `systemctl start flashpods-api`
- [ ] Verify health: `curl http://10.0.0.1:8080/health`

## Phase 4: Validation Tests

### SPIRE Attestation

- [ ] Run worker attestation test:
  ```bash
  podman run --rm \
    --label flashpods-job-type=worker \
    -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
    alpine \
    spire-agent api fetch x509 -socketPath /run/spire/sockets/agent.sock
  ```
  Expected: SVID with `spiffe://flashpods.local/worker`

- [ ] Run agent attestation test:
  ```bash
  podman run --rm \
    --label flashpods-job-type=agent \
    -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
    alpine \
    spire-agent api fetch x509 -socketPath /run/spire/sockets/agent.sock
  ```
  Expected: SVID with `spiffe://flashpods.local/agent`

### Token Exchange

- [ ] Run token exchange test:
  ```bash
  podman run --rm \
    --label flashpods-job-type=agent \
    -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
    -v /run/flashpods/token.sock:/run/flashpods/token.sock:ro \
    alpine sh -c '
      apk add --no-cache curl jq spire
      JWT=$(spire-agent api fetch jwt -audience flashpods-token-service -socketPath /run/spire/sockets/agent.sock | jq -r ".[].svids[0].svid")
      curl -s --unix-socket /run/flashpods/token.sock -H "Authorization: Bearer $JWT" http://localhost/tokens
    '
  ```
  Expected: JSON with ANTHROPIC_API_KEY and GITHUB_TOKEN

### Firewall

- [ ] Run firewall test:
  ```bash
  podman run --rm alpine sh -c '
    apk add --no-cache curl
    echo "Testing API access (should fail)..."
    curl -v --connect-timeout 5 http://10.0.0.1:8080/health 2>&1 || echo "PASS: API blocked"
    echo "Testing internet access (should work)..."
    curl -v --connect-timeout 5 https://api.anthropic.com/ 2>&1 | head -5
  '
  ```
  Expected: API blocked, internet accessible

### End-to-End

- [ ] Test job creation from laptop:
  ```bash
  curl -X POST http://10.0.0.1:8080/jobs \
    -H "Authorization: Bearer $FLASHPODS_API_TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"type":"worker","command":"echo hello > /artifacts/test.txt","image":"alpine:latest"}'
  ```
- [ ] Verify job completes
- [ ] Verify artifact is retrievable

## Phase 5: MCP Server

### On Laptop

- [ ] Install MCP server dependencies
- [ ] Configure environment:
  ```bash
  export FLASHPODS_API_TOKEN="<token>"
  ```
- [ ] Configure Claude Desktop to use MCP server
- [ ] Test spawn_worker tool
- [ ] Test get_job_status tool
- [ ] Test get_job_artifacts tool

## Post-Deployment

### Monitoring Setup

- [ ] Configure log aggregation
- [ ] Set up disk usage alerts
- [ ] Set up service health alerts
- [ ] Configure backup schedule

### Documentation

- [ ] Record all generated keys/tokens securely
- [ ] Document any deviations from spec
- [ ] Create runbook for common issues

## Rollback Plan

If deployment fails:

1. Stop all Flashpods services
2. Revert NixOS configuration: `nixos-rebuild switch --rollback`
3. Restore database from backup if corrupted
4. Verify basic services (SSH, WireGuard) still work

## Sign-off

| Check | Verified By | Date |
|-------|-------------|------|
| All validation tests pass | | |
| API responds to authenticated requests | | |
| Jobs can be created and completed | | |
| Artifacts can be retrieved | | |
| Firewall blocks container API access | | |
| Token exchange works for both job types | | |
| MCP server can create jobs from laptop | | |

## Related Specs

- [NixOS Configuration](./13-nixos.md) - Infrastructure modules
- [Validation Tests](./14-validation.md) - Test details
- [Operations](./18-operations.md) - Troubleshooting guide
