# Operations Guide

## Monitoring

### Key Metrics

| Metric | Warning | Critical | Action |
|--------|---------|----------|--------|
| Disk usage (/tmp/flashpods) | >70% | >85% | Cleanup old uploads |
| Disk usage (/var/lib/flashpods) | >70% | >85% | Cleanup old artifacts |
| Disk usage (/var/log/flashpods) | >70% | >85% | Cleanup old logs |
| Running jobs | >8 | >10 | Check for stuck jobs |
| Failed jobs (last hour) | >5 | >10 | Investigate failures |
| API latency (p99) | >1s | >5s | Check database/Podman |
| SPIRE agent status | - | Unhealthy | Restart SPIRE agent |
| Token service status | - | Unhealthy | Restart token service |

### Health Checks

```bash
# API health
curl -s http://10.0.0.1:8080/health | jq

# SPIRE server
systemctl status spire-server

# SPIRE agent
systemctl status spire-agent

# Token service
systemctl status flashpods-token-service

# Podman socket
systemctl status podman-socket-flashpods

# Check running containers
podman ps --filter label=flashpods-job=true

# Check disk usage
df -h /tmp/flashpods /var/lib/flashpods /var/log/flashpods
```

### Log Locations

| Component | Log Location |
|-----------|--------------|
| Flashpods API | `journalctl -u flashpods-api` |
| Token Service | `journalctl -u flashpods-token-service` |
| SPIRE Server | `journalctl -u spire-server` |
| SPIRE Agent | `journalctl -u spire-agent` |
| Job logs | `/var/log/flashpods/{job_id}.log` |

## Alerting Rules

```yaml
# Example Prometheus alerting rules
groups:
  - name: flashpods
    rules:
      - alert: FlashpodsDiskUsageHigh
        expr: disk_usage_percent{mount=~"/tmp/flashpods|/var/lib/flashpods"} > 85
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Flashpods disk usage critical"

      - alert: FlashpodsAPIDown
        expr: up{job="flashpods-api"} == 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "Flashpods API is down"

      - alert: FlashpodsHighFailureRate
        expr: rate(flashpods_jobs_failed_total[1h]) > 10
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High job failure rate"
```

## Troubleshooting

### Jobs Stuck in "starting"

**Symptoms:** Job stays in `starting` state for >5 minutes

**Diagnosis:**
```bash
# Check container status
podman ps -a --filter label=flashpods-job-id=<job_id>

# Check Podman logs
journalctl -u podman-socket-flashpods --since "5 minutes ago"

# Check if image pull is happening
podman events --filter event=pull --since "5m"
```

**Common causes:**
1. Image pull taking too long (large image, slow network)
2. Podman socket unresponsive
3. Resource limits preventing container start

**Resolution:**
```bash
# Kill stuck container
podman rm -f <container_id>

# Mark job as failed
sqlite3 /var/lib/flashpods/db/flashpods.db \
  "UPDATE jobs SET status='failed', error='stuck_in_starting' WHERE id='<job_id>'"
```

### SPIRE Attestation Failing

**Symptoms:** Containers can't get identity, token exchange fails

**Diagnosis:**
```bash
# Check SPIRE agent logs
journalctl -u spire-agent --since "10 minutes ago" | grep -i error

# Test attestation manually
podman run --rm \
  --label flashpods-job-type=worker \
  -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
  alpine \
  spire-agent api fetch x509 -socketPath /run/spire/sockets/agent.sock

# Check Podman socket access
ls -la /run/flashpods/podman.sock
```

**Common causes:**
1. SPIRE agent can't read Podman socket (permissions)
2. Cgroup matcher regex doesn't match
3. SPIRE entries not registered

**Resolution:**
```bash
# Restart SPIRE agent
systemctl restart spire-agent

# Re-register entries
spire-server entry create \
  -spiffeID spiffe://flashpods.local/worker \
  -parentID spiffe://flashpods.local/host \
  -selector docker:label:flashpods-job-type:worker
```

### Token Service Not Responding

**Symptoms:** Containers timeout on token exchange

**Diagnosis:**
```bash
# Check service status
systemctl status flashpods-token-service

# Check socket exists
ls -la /run/flashpods/token.sock

# Test manually
curl --unix-socket /run/flashpods/token.sock http://localhost/health
```

**Common causes:**
1. Secrets file missing or unreadable
2. SPIRE trust bundle not loaded
3. Service crashed

**Resolution:**
```bash
# Check secrets file
cat /etc/flashpods/secrets.json

# Restart service
systemctl restart flashpods-token-service
```

### Database Issues

**Symptoms:** API returns 500 errors, slow queries

**Diagnosis:**
```bash
# Check database integrity
sqlite3 /var/lib/flashpods/db/flashpods.db "PRAGMA integrity_check"

# Check database size
ls -lh /var/lib/flashpods/db/flashpods.db

# Check for locked database
lsof /var/lib/flashpods/db/flashpods.db
```

**Resolution:**
```bash
# Vacuum database (reclaim space)
sqlite3 /var/lib/flashpods/db/flashpods.db "VACUUM"

# If corrupted, restore from backup
cp /var/lib/flashpods/db/flashpods.db.bak /var/lib/flashpods/db/flashpods.db
systemctl restart flashpods-api
```

### WireGuard Connectivity Issues

**Symptoms:** MCP server can't reach API

**Diagnosis:**
```bash
# Check WireGuard interface
wg show wg0

# Ping server
ping 10.0.0.1

# Check firewall
nft list ruleset
```

**Resolution:**
```bash
# Restart WireGuard
systemctl restart wg-quick@wg0

# Re-add peer (if needed)
wg set wg0 peer <pubkey> allowed-ips 10.0.0.2/32
```

## Backup & Recovery

### Database Backup

```bash
# Create backup
sqlite3 /var/lib/flashpods/db/flashpods.db ".backup /var/lib/flashpods/db/backup-$(date +%Y%m%d).db"

# Automated daily backup (add to cron)
0 2 * * * sqlite3 /var/lib/flashpods/db/flashpods.db ".backup /var/backups/flashpods/db-$(date +\%Y\%m\%d).db"
```

### Disaster Recovery

**Complete failure scenario:**

1. Provision new NixOS server
2. Apply Flashpods NixOS configuration
3. Restore database from backup
4. Restore secrets file from secure storage
5. Generate new WireGuard keys and update client
6. Register SPIRE entries
7. Run validation tests

### Rolling Upgrade

1. Drain running jobs (wait for completion or cancel)
2. Stop Flashpods API: `systemctl stop flashpods-api`
3. Apply new NixOS configuration: `nixos-rebuild switch`
4. Run database migrations (if any)
5. Start Flashpods API: `systemctl start flashpods-api`
6. Verify health: `curl http://10.0.0.1:8080/health`
7. Run validation tests

## Maintenance Tasks

### Cleanup Stale Data

```bash
# Manual cleanup of old jobs (>7 days)
sqlite3 /var/lib/flashpods/db/flashpods.db \
  "UPDATE jobs SET status='cleaning' WHERE status IN ('completed','failed','timed_out','cancelled') AND completed_at < datetime('now', '-7 days')"

# Trigger cleanup daemon
systemctl restart flashpods-cleanup
```

### Pre-pull Images

```bash
# Pull common images to avoid first-job delay
for img in rust:latest node:20 python:3.12 ubuntu:22.04; do
  podman pull $img
done
```

### Rotate Bearer Token

1. Generate new token: `openssl rand -hex 32`
2. Update server: `echo "<new_token>" > /etc/flashpods/api-token && systemctl restart flashpods-api`
3. Update MCP server config on laptop
4. Verify connectivity

## Performance Tuning

### SQLite

```bash
# Add to API startup
sqlite3 /var/lib/flashpods/db/flashpods.db "PRAGMA journal_mode=WAL"
sqlite3 /var/lib/flashpods/db/flashpods.db "PRAGMA synchronous=NORMAL"
```

### Podman

```bash
# Increase container limits
echo "flashpods soft nofile 65536" >> /etc/security/limits.conf
echo "flashpods hard nofile 65536" >> /etc/security/limits.conf
```

## Related Specs

- [Validation Tests](./14-validation.md) - System validation
- [Deployment](./19-deployment.md) - Deployment checklist
- [NixOS Configuration](./13-nixos.md) - Infrastructure setup
