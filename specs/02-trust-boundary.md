# Trust Boundary & Security Model

## Authoritative Trust Model

| Source | Can Access API | Can Access rsync | Can Access Token Svc | Can Access Internet |
|--------|---------------|------------------|---------------------|---------------------|
| MCP Server (laptop) | Yes (bearer token) | Yes (IP allowlist) | N/A | N/A |
| Job containers | No (firewall) | No (firewall) | Yes (unix socket) | Yes |

## Critical Invariant

> **Job containers can never call `POST /jobs` or access rsync.**
>
> This is enforced by host firewall, not by convention.

This invariant guarantees that sub-agents cannot spawn more agents, regardless of what code runs inside them.

## Security Layers

| Layer | Mechanism |
|-------|-----------|
| Network | WireGuard |
| API access | Bearer token (MCP only) |
| Job spawning | Firewall blocks containers from API |
| File upload | rsyncd allowlist = laptop IP only |
| Identity | SPIRE workload attestation (kernel cgroups) |
| Secrets | Token exchange via SPIFFE ID |
| Container isolation | Rootless Podman |
| Privilege | Worker vs Agent via SPIFFE ID |

## Firewall Policy

| Port | Allowed Source |
|------|----------------|
| 51820 (WireGuard) | Internet |
| 8080 (API) | 10.0.0.2 only |
| 873 (rsync) | 10.0.0.2 only |
| token service | Unix socket only |

**Containers are blocked from 8080 and 873 by host firewall.**

## Container Network Access

| Destination | Allowed |
|-------------|---------|
| Internet (Anthropic, GitHub, npm) | Yes |
| API (10.0.0.1:8080) | No (firewall) |
| rsync (10.0.0.1:873) | No (firewall) |
| SPIRE socket | Yes (mounted) |
| Token socket | Yes (mounted) |

## Podman Execution Invariant

All containers are launched:
- **Rootless** (no root inside or outside container)
- **As host user `flashpods`**
- **With `--userns=keep-id`**

This ensures:
- Socket permissions work correctly
- Image storage is shared
- Cgroup layout is stable (required for SPIRE attestation)

## Firewall Implementation (nftables)

```nix
networking.nftables.tables.flashpods = {
  family = "inet";
  content = ''
    chain input {
      type filter hook input priority 0; policy accept;

      # Allow API and rsync only from WireGuard peer (laptop)
      iifname "wg0" tcp dport 8080 ip saddr 10.0.0.2 accept
      iifname "wg0" tcp dport 873 ip saddr 10.0.0.2 accept

      # Block API and rsync from Podman bridge interfaces
      iifname "podman*" tcp dport { 8080, 873 } drop
      iifname "cni-*" tcp dport { 8080, 873 } drop

      # Explicit drop for API and rsync from anywhere else
      tcp dport { 8080, 873 } drop
    }
  '';
};
```

**Why nftables + interface matching:**
- nftables is the modern Linux firewall (iptables successor)
- NixOS nftables integration is declarative and deterministic
- Interface matching (`iifname "podman*"`) works regardless of container subnet
- No hardcoded CIDRs that may change between Podman versions

## Related Specs

- [Container Mounts](./15-container-mounts.md) - Mount permissions
- [SPIRE](./08-spire.md) - Identity attestation
- [Token Service](./09-token-service.md) - Secret delivery
- [Validation Tests](./14-validation.md) - Firewall verification
