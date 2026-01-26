# Validation Tests

## Overview

These tests validate that the Flashpods deployment is correctly configured. All tests must pass before the system is considered production-ready.

## SPIRE Validation Test

Verifies that SPIRE workload attestation works correctly.

```bash
podman run --rm \
  --label flashpods-job-type=worker \
  -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
  alpine \
  spire-agent api fetch x509 -socketPath /run/spire/sockets/agent.sock
```

**Expected:** Returns SVID with `spiffe://flashpods.local/worker`

**Test for agent type:**

```bash
podman run --rm \
  --label flashpods-job-type=agent \
  -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
  alpine \
  spire-agent api fetch x509 -socketPath /run/spire/sockets/agent.sock
```

**Expected:** Returns SVID with `spiffe://flashpods.local/agent`

## Token Exchange Validation Test

Verifies that token exchange works correctly.

```bash
podman run --rm \
  --label flashpods-job-type=agent \
  -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
  -v /run/flashpods/token.sock:/run/flashpods/token.sock:ro \
  alpine sh -c '
    JWT=$(spire-agent api fetch jwt -audience flashpods-token-service -socketPath /run/spire/sockets/agent.sock | jq -r ".[].svids[0].svid")
    curl -s --unix-socket /run/flashpods/token.sock -H "Authorization: Bearer $JWT" http://localhost/tokens
  '
```

**Expected (agent):**
```json
{
  "ANTHROPIC_API_KEY": "sk-...",
  "GITHUB_TOKEN": "ghp_..."
}
```

**Test for worker (should not get ANTHROPIC_API_KEY):**

```bash
podman run --rm \
  --label flashpods-job-type=worker \
  -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
  -v /run/flashpods/token.sock:/run/flashpods/token.sock:ro \
  alpine sh -c '
    JWT=$(spire-agent api fetch jwt -audience flashpods-token-service -socketPath /run/spire/sockets/agent.sock | jq -r ".[].svids[0].svid")
    curl -s --unix-socket /run/flashpods/token.sock -H "Authorization: Bearer $JWT" http://localhost/tokens
  '
```

**Expected (worker):**
```json
{
  "ANTHROPIC_API_KEY": null,
  "GITHUB_TOKEN": "ghp_..."
}
```

## Firewall Validation Test

Verifies that containers cannot access API or rsync.

```bash
podman run --rm alpine sh -c '
  # Should fail (connection refused or timeout)
  echo "Testing API access..."
  curl -v --connect-timeout 5 http://10.0.0.1:8080/health 2>&1 || echo "API blocked: PASS"

  # Should fail
  echo "Testing rsync access..."
  curl -v --connect-timeout 5 rsync://10.0.0.1/uploads/ 2>&1 || echo "rsync blocked: PASS"

  # Should succeed
  echo "Testing internet access..."
  curl -v --connect-timeout 5 https://api.anthropic.com/ 2>&1 | head -20
'
```

**Expected:**
- API access: Connection refused or timeout
- rsync access: Connection refused or timeout
- Internet access: Successfully connects (may return 401 without auth)

## WireGuard Connectivity Test

Run from laptop:

```bash
# Test API endpoint
curl -H "Authorization: Bearer $FLASHPODS_API_TOKEN" http://10.0.0.1:8080/health

# Test rsync
rsync rsync://10.0.0.1/uploads/
```

**Expected:**
- API returns `{"status": "healthy", ...}`
- rsync shows module listing

## End-to-End Job Test

```bash
# From laptop, via MCP or direct API call
curl -X POST http://10.0.0.1:8080/jobs \
  -H "Authorization: Bearer $FLASHPODS_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "type": "worker",
    "command": "echo hello > /artifacts/test.txt && cat /artifacts/test.txt",
    "image": "alpine:latest"
  }'
```

**Expected:**
- Job created successfully
- Job completes with exit code 0
- Artifact `test.txt` contains "hello"

## Container Isolation Test

Verify containers run rootless:

```bash
podman run --rm alpine id
```

**Expected:** Shows non-root user (e.g., `uid=1000(flashpods)`)

## Socket Permission Test

```bash
# Check socket ownership
ls -la /run/flashpods/
ls -la /run/spire/sockets/
```

**Expected:**
```
/run/flashpods/:
srw-rw---- flashpods flashpods podman.sock
srw-rw---- flashpods flashpods token.sock

/run/spire/sockets/:
srwxrwxrwx root root agent.sock
```

## Test Checklist

| Test | Command | Expected Result |
|------|---------|-----------------|
| SPIRE worker attestation | See above | SVID: worker |
| SPIRE agent attestation | See above | SVID: agent |
| Token exchange (agent) | See above | Both tokens |
| Token exchange (worker) | See above | Only GITHUB_TOKEN |
| Firewall blocks API | curl from container | Timeout/refused |
| Firewall blocks rsync | curl from container | Timeout/refused |
| Internet allowed | curl from container | Success |
| WireGuard API | curl from laptop | 200 OK |
| WireGuard rsync | rsync from laptop | Module list |
| E2E job | Create + complete | Exit 0, artifact |
| Rootless | id in container | Non-root UID |
| Socket perms | ls -la | Correct ownership |

## Related Specs

- [Trust Boundary](./02-trust-boundary.md) - Security model
- [SPIRE](./08-spire.md) - Identity system
- [Token Service](./09-token-service.md) - Token exchange
