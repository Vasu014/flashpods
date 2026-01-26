# NixOS Configuration

## flake.nix Structure

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
  };

  outputs = { self, nixpkgs }: {
    nixosConfigurations.flashpods = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        ./modules/wireguard.nix
        ./modules/firewall.nix
        ./modules/podman.nix
        ./modules/rsyncd.nix
        ./modules/spire.nix
        ./modules/flashpods-api.nix
        ./modules/token-service.nix
      ];
    };
  };
}
```

## Firewall Module

```nix
# modules/firewall.nix
{ ... }: {
  networking.nftables.enable = true;

  networking.firewall = {
    enable = true;
    allowedUDPPorts = [ 51820 ];  # WireGuard
  };

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
}
```

## Podman Module

```nix
# modules/podman.nix
{ pkgs, ... }: {
  virtualisation.podman = {
    enable = true;
    dockerCompat = true;
  };

  users.groups.flashpods = {};

  users.users.flashpods = {
    isNormalUser = true;
    group = "flashpods";
    # Enable lingering for user services
    linger = true;
  };

  # Podman socket for SPIRE agent
  systemd.services.podman-socket-flashpods = {
    description = "Podman API socket for Flashpods";
    after = [ "network.target" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "simple";
      User = "flashpods";
      Group = "flashpods";
      ExecStart = "${pkgs.podman}/bin/podman system service --time=0 unix:///run/flashpods/podman.sock";
      Restart = "always";
      RestartSec = "5s";
    };
  };

  systemd.tmpfiles.rules = [
    "d /run/flashpods 0750 flashpods flashpods -"
  ];

  # Pre-pull common images (run as flashpods user)
  system.activationScripts.pullImages = ''
    su - flashpods -c '${pkgs.podman}/bin/podman pull rust:latest' || true
    su - flashpods -c '${pkgs.podman}/bin/podman pull rust:1.75' || true
    su - flashpods -c '${pkgs.podman}/bin/podman pull node:20' || true
    su - flashpods -c '${pkgs.podman}/bin/podman pull node:18' || true
    su - flashpods -c '${pkgs.podman}/bin/podman pull python:3.12' || true
    su - flashpods -c '${pkgs.podman}/bin/podman pull python:3.11' || true
    su - flashpods -c '${pkgs.podman}/bin/podman pull ubuntu:24.04' || true
    su - flashpods -c '${pkgs.podman}/bin/podman pull ubuntu:22.04' || true
    su - flashpods -c '${pkgs.podman}/bin/podman pull alpine:latest' || true
  '';
}
```

## WireGuard Module

```nix
# modules/wireguard.nix
{ ... }: {
  networking.wireguard.interfaces.wg0 = {
    ips = [ "10.0.0.1/24" ];
    listenPort = 51820;
    privateKeyFile = "/etc/wireguard/private.key";

    peers = [{
      publicKey = "YOUR_LAPTOP_PUBLIC_KEY";  # Replace with actual key
      allowedIPs = [ "10.0.0.2/32" ];
    }];
  };
}
```

## rsync Daemon Module

```nix
# modules/rsyncd.nix
{ ... }: {
  services.rsyncd = {
    enable = true;
    settings = {
      global = {
        address = "10.0.0.1";
        "hosts allow" = "10.0.0.2";  # Laptop only
        "hosts deny" = "*";
        "max connections" = 10;
        "use chroot" = true;
        timeout = 300;
      };
      uploads = {
        path = "/tmp/flashpods/uploads";
        "read only" = false;
        uid = "flashpods";
        gid = "flashpods";
      };
    };
  };

  systemd.tmpfiles.rules = [
    "d /tmp/flashpods/uploads 0755 flashpods flashpods -"
    "d /var/lib/flashpods 0755 flashpods flashpods -"
    "d /var/lib/flashpods/artifacts 0755 flashpods flashpods -"
    "d /var/lib/flashpods/db 0755 flashpods flashpods -"
    "d /var/log/flashpods 0755 flashpods flashpods -"
  ];
}
```

## SPIRE Module

```nix
# modules/spire.nix
{ pkgs, ... }: {
  # SPIRE Server
  systemd.services.spire-server = {
    description = "SPIRE Server";
    after = [ "network.target" ];
    wantedBy = [ "multi-user.target" ];

    serviceConfig = {
      Type = "simple";
      ExecStart = "${pkgs.spire}/bin/spire-server run -config /etc/spire/server.conf";
      Restart = "always";
      RestartSec = "5s";
    };
  };

  # SPIRE Agent (runs as root for cgroup access, flashpods group for socket)
  systemd.services.spire-agent = {
    description = "SPIRE Agent";
    after = [ "network.target" "spire-server.service" "podman-socket-flashpods.service" ];
    requires = [ "podman-socket-flashpods.service" ];
    wantedBy = [ "multi-user.target" ];

    serviceConfig = {
      Type = "simple";
      ExecStart = "${pkgs.spire}/bin/spire-agent run -config /etc/spire/agent.conf";
      Restart = "always";
      RestartSec = "5s";
      SupplementaryGroups = [ "flashpods" ];
    };
  };

  systemd.tmpfiles.rules = [
    "d /etc/spire 0755 root root -"
    "d /var/lib/spire 0700 root root -"
    "d /var/lib/spire/server 0700 root root -"
    "d /var/lib/spire/agent 0700 root root -"
    "d /run/spire 0755 root root -"
    "d /run/spire/sockets 0755 root root -"
  ];
}
```

## Flashpods API Module

```nix
# modules/flashpods-api.nix
{ pkgs, ... }: {
  systemd.services.flashpods-api = {
    description = "Flashpods API Server";
    after = [
      "network.target"
      "podman-socket-flashpods.service"
      "spire-agent.service"
    ];
    requires = [ "podman-socket-flashpods.service" ];
    wantedBy = [ "multi-user.target" ];

    environment = {
      FLASHPODS_DB_PATH = "/var/lib/flashpods/db/flashpods.db";
      FLASHPODS_UPLOADS_PATH = "/tmp/flashpods/uploads";
      FLASHPODS_ARTIFACTS_PATH = "/var/lib/flashpods/artifacts";
      FLASHPODS_LOGS_PATH = "/var/log/flashpods";
      FLASHPODS_LISTEN_ADDR = "10.0.0.1:8080";
      FLASHPODS_TOKEN_FILE = "/etc/flashpods/api-token";
      FLASHPODS_PODMAN_SOCKET = "/run/flashpods/podman.sock";
      RUST_LOG = "info";
    };

    serviceConfig = {
      Type = "simple";
      User = "flashpods";
      Group = "flashpods";
      ExecStart = "/usr/local/bin/flashpods-api";
      Restart = "always";
      RestartSec = "5s";

      # Security hardening
      NoNewPrivileges = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      PrivateTmp = false;  # Need access to /tmp/flashpods
      ReadWritePaths = [
        "/var/lib/flashpods"
        "/var/log/flashpods"
        "/tmp/flashpods"
      ];
      ReadOnlyPaths = [
        "/etc/flashpods"
        "/run/flashpods"
      ];
    };
  };

  # Ensure secrets directory exists
  systemd.tmpfiles.rules = [
    "d /etc/flashpods 0700 root root -"
    "f /etc/flashpods/api-token 0600 root root -"
  ];
}
```

## Token Service Module

```nix
# modules/token-service.nix
{ pkgs, ... }: {
  systemd.services.flashpods-token-service = {
    description = "Flashpods Token Exchange Service";
    after = [
      "network.target"
      "spire-agent.service"
    ];
    requires = [ "spire-agent.service" ];
    wantedBy = [ "multi-user.target" ];

    environment = {
      FLASHPODS_SECRETS_FILE = "/etc/flashpods/secrets.json";
      FLASHPODS_SOCKET_PATH = "/run/flashpods/token.sock";
      FLASHPODS_SPIRE_SOCKET = "/run/spire/sockets/agent.sock";
      RUST_LOG = "info";
    };

    serviceConfig = {
      Type = "simple";
      # Run as root to read secrets, but create socket owned by flashpods
      ExecStart = "/usr/local/bin/flashpods-token-service";
      Restart = "always";
      RestartSec = "5s";

      # Security hardening
      NoNewPrivileges = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      PrivateTmp = true;
      ReadOnlyPaths = [
        "/etc/flashpods"
        "/run/spire"
      ];
      ReadWritePaths = [
        "/run/flashpods"
      ];
    };

    # Set socket ownership after service starts
    postStart = ''
      sleep 1
      chown flashpods:flashpods /run/flashpods/token.sock || true
      chmod 0660 /run/flashpods/token.sock || true
    '';
  };

  # Ensure secrets file exists with correct permissions
  systemd.tmpfiles.rules = [
    "f /etc/flashpods/secrets.json 0600 root root -"
  ];
}
```

## Service Startup Order

```
network.target
     │
     ├──► podman-socket-flashpods.service
     │           │
     ├──► spire-server.service
     │           │
     │           ▼
     │    spire-agent.service (requires podman socket)
     │           │
     ├───────────┴──────────┐
     │                      │
     ▼                      ▼
flashpods-api.service    flashpods-token-service.service
```

## Socket Permissions

| Path | Owner | Group | Mode | Purpose |
|------|-------|-------|------|---------|
| /run/flashpods/ | flashpods | flashpods | 0750 | Socket directory |
| /run/flashpods/podman.sock | flashpods | flashpods | 0660 | Podman API socket |
| /run/flashpods/token.sock | flashpods | flashpods | 0660 | Token service socket |
| /run/spire/sockets/agent.sock | root | root | 0777 | SPIRE Workload API |

## Directory Structure

```
/tmp/flashpods/uploads/              Temporary file uploads
/var/lib/flashpods/artifacts/        Job artifacts
/var/lib/flashpods/db/               SQLite database
/var/log/flashpods/                  Job logs
/etc/flashpods/secrets.json          API keys (0600 root:root)
/etc/flashpods/api-token             API bearer token (0600 root:root)
/run/flashpods/token.sock            Token service socket
/run/flashpods/podman.sock           Podman socket for SPIRE
/run/spire/sockets/agent.sock        SPIRE workload API
/etc/spire/server.conf               SPIRE server config
/etc/spire/agent.conf                SPIRE agent config
/var/lib/spire/server/               SPIRE server data
/var/lib/spire/agent/                SPIRE agent data
```

## Configuration Files

### /etc/flashpods/secrets.json

```json
{
  "anthropic_api_key": "sk-ant-...",
  "github_token": "ghp_..."
}
```

### /etc/flashpods/api-token

```
<64-character-hex-token>
```

Generate with: `openssl rand -hex 32`

## Related Specs

- [Trust Boundary](./02-trust-boundary.md) - Firewall rules explained
- [SPIRE](./08-spire.md) - SPIRE configuration details
- [Token Service](./09-token-service.md) - Token service setup
- [Deployment](./19-deployment.md) - Deployment checklist
