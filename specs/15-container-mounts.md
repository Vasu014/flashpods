# Container Mounts

## Mount Configuration

| Path | Purpose | Worker Mode | Agent Mode |
|------|---------|-------------|------------|
| /work | Job files | ro | rw |
| /artifacts | Outputs | rw | rw |
| /run/spire/sockets/agent.sock | SPIRE Workload API | ro | ro |
| /run/flashpods/token.sock | Token exchange | ro | ro |

## /work Mount Semantics

Flashpods enforces different `/work` mount modes based on job type.

### Workers

- Receive a **read-only** view of uploaded files
- Cannot mutate source code
- All outputs must go through `/artifacts`

This guarantees:
- Deterministic execution
- Reproducible builds/tests
- No hidden side effects

### Agents

- Receive a **read-write** workspace
- May mutate files locally
- Must publish all authoritative changes via git commits

Local filesystem changes inside agent containers are **non-authoritative** and become real only after passing through:

```
git commit → git push → review → merge
```

## System Guarantee

> The source of truth for code changes is always git, never the container filesystem.

Agent `/work` mutations are ephemeral drafts. Without `git push`, they disappear when the container dies.

## Complete Podman Command Template

### Worker

```bash
podman run \
  --rm \
  --name "job_${JOB_ID}" \
  --label flashpods-job=true \
  --label "flashpods-job-id=${JOB_ID}" \
  --label flashpods-job-type=worker \
  --cpus "${CPUS}" \
  --memory "${MEMORY_GB}g" \
  --userns=keep-id \
  --network=slirp4netns \
  --security-opt no-new-privileges \
  --cap-drop ALL \
  -v "/tmp/flashpods/uploads/${UPLOAD_ID}:/work:ro" \
  -v "/var/lib/flashpods/artifacts/${JOB_ID}:/artifacts:rw" \
  -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
  -v /run/flashpods/token.sock:/run/flashpods/token.sock:ro \
  "${IMAGE}" \
  /bin/sh -c "${COMMAND}"
```

### Agent

```bash
podman run \
  --rm \
  --name "job_${JOB_ID}" \
  --label flashpods-job=true \
  --label "flashpods-job-id=${JOB_ID}" \
  --label flashpods-job-type=agent \
  --cpus "${CPUS}" \
  --memory "${MEMORY_GB}g" \
  --userns=keep-id \
  --network=slirp4netns \
  --security-opt no-new-privileges \
  --cap-drop ALL \
  -e "FLASHPODS_TASK=${TASK}" \
  -e "FLASHPODS_CONTEXT=${CONTEXT}" \
  -e "FLASHPODS_GIT_BRANCH=${GIT_BRANCH}" \
  -e "FLASHPODS_JOB_ID=${JOB_ID}" \
  -v "/tmp/flashpods/uploads/${UPLOAD_ID}:/work:rw" \
  -v "/var/lib/flashpods/artifacts/${JOB_ID}:/artifacts:rw" \
  -v /run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro \
  -v /run/flashpods/token.sock:/run/flashpods/token.sock:ro \
  flashpods/agent:latest \
  /entrypoint.sh
```

## Flag Explanations

| Flag | Purpose |
|------|---------|
| `--rm` | Remove container after exit |
| `--name` | Container name for identification |
| `--label` | Metadata for SPIRE attestation and reconciliation |
| `--cpus` | Hard CPU limit (throttled, not killed) |
| `--memory` | Hard memory limit (OOM killed on exceed) |
| `--userns=keep-id` | Map container UID to host flashpods user |
| `--network=slirp4netns` | User-mode networking (rootless) |
| `--security-opt no-new-privileges` | Prevent privilege escalation |
| `--cap-drop ALL` | Drop all Linux capabilities |
| `-v :ro` | Read-only mount |
| `-v :rw` | Read-write mount |
| `-e` | Environment variable |

## Container Labels

All containers receive these labels for identification and SPIRE attestation:

| Label | Value | Purpose |
|-------|-------|---------|
| `flashpods-job` | `true` | Identifies Flashpods-managed containers |
| `flashpods-job-id` | `job_xyz789` | Links container to job record |
| `flashpods-job-type` | `worker` or `agent` | SPIRE attestation selector |

## userns Configuration

All containers run with `--userns=keep-id`:

- Maps container UID to host `flashpods` user
- Files created in `/artifacts` owned by flashpods
- Ensures socket permissions work correctly
- Required for stable cgroup layout (SPIRE attestation)

## Network Configuration

```
--network=slirp4netns
```

This provides:
- User-mode networking (no root required)
- Outbound internet access
- Container-to-host isolation
- No inter-container communication
- DNS resolution via host

Containers **cannot**:
- Access host ports directly
- Communicate with other containers
- Access API or rsync (blocked by firewall)

Containers **can**:
- Make outbound connections to internet
- Access mounted sockets (SPIRE, token service)

## Resource Limits

Enforced via Podman flags:

```bash
--cpus <n>      # CPU limit (hard throttling)
--memory <n>g   # Memory limit (OOM kill on exceed)
```

**CPU behavior:**
- Container is throttled when exceeding CPU limit
- Not killed, just slowed down

**Memory behavior:**
- Container is killed (SIGKILL) when exceeding limit
- Exit code 137 (128 + 9)
- Job marked `failed` with `oom_killed` error

## Security Hardening

```bash
--security-opt no-new-privileges  # Prevent setuid, capabilities
--cap-drop ALL                    # Drop all capabilities
```

Containers have no Linux capabilities. They cannot:
- Change user ID
- Mount filesystems
- Use raw sockets
- Load kernel modules
- Access hardware directly

## /artifacts Directory

Created by Flashpods before container start:

```bash
mkdir -p "/var/lib/flashpods/artifacts/${JOB_ID}"
chmod 0755 "/var/lib/flashpods/artifacts/${JOB_ID}"
chown flashpods:flashpods "/var/lib/flashpods/artifacts/${JOB_ID}"
```

Inside container, `/artifacts`:
- Is writable
- Allows subdirectories
- Allows symlinks (validated on copy)
- Max 2 GB total
- Max 200 files

## DNS Configuration

Containers inherit host DNS configuration. Podman copies `/etc/resolv.conf` into container.

If DNS issues occur, check:
```bash
# On host
cat /etc/resolv.conf

# In container (debug)
podman run --rm alpine cat /etc/resolv.conf
```

## Init Process

For simple commands, no init process manager is needed. For complex jobs that spawn subprocesses:

```bash
# Add tini for proper signal handling and zombie reaping
podman run --init ...
```

Or include tini in the image:

```dockerfile
RUN apk add --no-cache tini
ENTRYPOINT ["/sbin/tini", "--"]
```

## Signal Handling

When killing a job:

1. `SIGTERM` sent to container PID 1
2. Grace period (10 seconds)
3. `SIGKILL` sent to entire cgroup

Container PID 1 should handle `SIGTERM` for graceful cleanup. After grace period, all processes in the container are forcibly killed.

## Implementation Example (Rust)

```rust
use std::process::Command;

fn create_container(job: &Job, upload: &Upload) -> Result<String, Error> {
    let work_mode = match job.job_type {
        JobType::Worker => "ro",
        JobType::Agent => "rw",
    };

    let mut cmd = Command::new("podman");
    cmd.args(&["run", "-d", "--rm"]);
    cmd.args(&["--name", &format!("job_{}", job.id)]);
    cmd.args(&["--label", "flashpods-job=true"]);
    cmd.args(&["--label", &format!("flashpods-job-id={}", job.id)]);
    cmd.args(&["--label", &format!("flashpods-job-type={}", job.job_type)]);
    cmd.args(&["--cpus", &job.cpus.to_string()]);
    cmd.args(&["--memory", &format!("{}g", job.memory_gb)]);
    cmd.args(&["--userns=keep-id"]);
    cmd.args(&["--network=slirp4netns"]);
    cmd.args(&["--security-opt", "no-new-privileges"]);
    cmd.args(&["--cap-drop", "ALL"]);

    // Mounts
    cmd.args(&["-v", &format!(
        "/tmp/flashpods/uploads/{}:/work:{}",
        upload.id, work_mode
    )]);
    cmd.args(&["-v", &format!(
        "/var/lib/flashpods/artifacts/{}:/artifacts:rw",
        job.id
    )]);
    cmd.args(&["-v", "/run/spire/sockets/agent.sock:/run/spire/sockets/agent.sock:ro"]);
    cmd.args(&["-v", "/run/flashpods/token.sock:/run/flashpods/token.sock:ro"]);

    // Environment variables for agents
    if job.job_type == JobType::Agent {
        cmd.args(&["-e", &format!("FLASHPODS_TASK={}", job.task.as_ref().unwrap())]);
        if let Some(context) = &job.context {
            cmd.args(&["-e", &format!("FLASHPODS_CONTEXT={}", context)]);
        }
        cmd.args(&["-e", &format!("FLASHPODS_GIT_BRANCH={}", job.git_branch.as_ref().unwrap())]);
    }

    // Image and command
    cmd.arg(&job.image);

    match job.job_type {
        JobType::Worker => {
            cmd.args(&["/bin/sh", "-c", job.command.as_ref().unwrap()]);
        }
        JobType::Agent => {
            cmd.arg("/entrypoint.sh");
        }
    }

    let output = cmd.output()?;
    if !output.status.success() {
        return Err(Error::ContainerStart(String::from_utf8_lossy(&output.stderr).to_string()));
    }

    let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(container_id)
}
```

## Related Specs

- [Jobs](./03-jobs.md) - Job types and their semantics
- [Trust Boundary](./02-trust-boundary.md) - Security model
- [SPIRE](./08-spire.md) - How labels enable attestation
- [Artifacts](./07-artifacts.md) - /artifacts directory usage
- [Token Service](./09-token-service.md) - Agent entrypoint details
