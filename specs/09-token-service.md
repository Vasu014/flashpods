# Token Exchange Service

## Architecture

Token Service listens **only** on unix socket:
```
/run/flashpods/token.sock
```

**No TCP listener.** Containers access it via mounted socket only.

## Token Exchange Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Token Exchange Flow                                │
│                                                                              │
│  1. Container fetches JWT-SVID from SPIRE:                                  │
│                                                                              │
│     spire-agent api fetch jwt \                                             │
│       -socketPath /run/spire/sockets/agent.sock \                           │
│       -audience flashpods-token-service                                     │
│                                                                              │
│  2. Container calls token service via unix socket:                          │
│                                                                              │
│     curl --unix-socket /run/flashpods/token.sock \                          │
│       -H "Authorization: Bearer <JWT-SVID>" \                               │
│       http://localhost/tokens                                                │
│                                                                              │
│  3. Token service:                                                          │
│     a. Verifies JWT signature (SPIRE trust bundle)                          │
│     b. Verifies audience = "flashpods-token-service"                        │
│     c. Extracts SPIFFE ID from JWT subject                                  │
│     d. Maps SPIFFE ID to token scope                                        │
│     e. Returns tokens as JSON                                               │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Token Policy

| SPIFFE ID | ANTHROPIC_API_KEY | GITHUB_TOKEN |
|-----------|-------------------|--------------|
| `spiffe://flashpods.local/agent` | Yes | Yes |
| `spiffe://flashpods.local/worker` | No | Yes |

**Token service ignores any caller-supplied job type.** Only the cryptographically-verified SPIFFE ID matters.

## Response Format

**Success (200):**
```json
{
  "ANTHROPIC_API_KEY": "sk-ant-...",
  "GITHUB_TOKEN": "ghp_..."
}
```

**Worker response (ANTHROPIC_API_KEY omitted):**
```json
{
  "ANTHROPIC_API_KEY": null,
  "GITHUB_TOKEN": "ghp_..."
}
```

**Error responses:**

| Status | Error Code | Condition |
|--------|------------|-----------|
| 400 | missing_authorization | No Authorization header |
| 401 | invalid_jwt | JWT signature verification failed |
| 401 | invalid_audience | JWT audience != "flashpods-token-service" |
| 401 | expired_jwt | JWT has expired |
| 403 | unknown_identity | SPIFFE ID not in policy |
| 500 | secrets_unavailable | Secrets file unreadable |

## Implementation

```rust
use axum::{Router, routing::get, extract::Extension};
use hyper_unix_connector::UnixConnector;

// Listens on /run/flashpods/token.sock

async fn handle_request(
    headers: HeaderMap,
    Extension(config): Extension<Arc<Config>>,
) -> Result<Json<Tokens>, Error> {
    // 1. Extract JWT from "Bearer <jwt>"
    let auth = headers.get("Authorization")
        .ok_or(Error::MissingAuthorization)?
        .to_str()?;

    let jwt = auth.strip_prefix("Bearer ")
        .ok_or(Error::MissingBearer)?;

    // 2. Verify JWT signature using SPIRE trust bundle
    let claims = verify_jwt(jwt, &config.spire_trust_bundle)?;

    // 3. Check audience
    if !claims.aud.contains(&"flashpods-token-service".to_string()) {
        return Err(Error::InvalidAudience);
    }

    // 4. Check expiration
    if claims.exp < Utc::now().timestamp() {
        return Err(Error::ExpiredJwt);
    }

    // 5. Extract SPIFFE ID and return appropriate tokens
    match claims.sub.as_str() {
        "spiffe://flashpods.local/agent" => Ok(Json(Tokens {
            anthropic_api_key: Some(config.secrets.anthropic_api_key.clone()),
            github_token: Some(config.secrets.github_token.clone()),
        })),
        "spiffe://flashpods.local/worker" => Ok(Json(Tokens {
            anthropic_api_key: None,
            github_token: Some(config.secrets.github_token.clone()),
        })),
        _ => Err(Error::UnknownIdentity),
    }
}

// Startup validation
fn load_secrets() -> Result<Secrets, Error> {
    let path = "/etc/flashpods/secrets.json";
    let content = fs::read_to_string(path)
        .map_err(|e| {
            error!("FATAL: Cannot read secrets file {}: {}", path, e);
            Error::SecretsUnavailable
        })?;

    serde_json::from_str(&content)
        .map_err(|e| {
            error!("FATAL: Invalid secrets JSON: {}", e);
            Error::SecretsUnavailable
        })
}
```

## Container Entrypoint (Agent)

```bash
#!/bin/bash
set -e

SPIRE_SOCKET="/run/spire/sockets/agent.sock"
TOKEN_SOCKET="/run/flashpods/token.sock"
MAX_RETRIES=3
RETRY_DELAY=2

# 1. Wait for SPIRE socket (max 10 seconds)
echo "Waiting for SPIRE socket..."
for i in $(seq 1 10); do
    if [ -S "$SPIRE_SOCKET" ]; then
        break
    fi
    if [ $i -eq 10 ]; then
        echo "ERROR: SPIRE socket not available after 10 seconds"
        exit 1
    fi
    sleep 1
done

# 2. Fetch JWT-SVID from SPIRE (with retry)
echo "Fetching SPIRE JWT..."
for attempt in $(seq 1 $MAX_RETRIES); do
    JWT=$(spire-agent api fetch jwt \
        -socketPath "$SPIRE_SOCKET" \
        -audience flashpods-token-service \
        -timeout 10s 2>/dev/null | jq -r '.[] | .svids[0].svid' 2>/dev/null) || true

    if [ -n "$JWT" ] && [ "$JWT" != "null" ]; then
        break
    fi

    if [ $attempt -eq $MAX_RETRIES ]; then
        echo "ERROR: Failed to fetch SPIRE JWT after $MAX_RETRIES attempts"
        exit 1
    fi

    echo "Retry $attempt/$MAX_RETRIES in ${RETRY_DELAY}s..."
    sleep $RETRY_DELAY
done

# 3. Exchange JWT for API tokens via unix socket (with retry)
echo "Exchanging JWT for tokens..."
for attempt in $(seq 1 $MAX_RETRIES); do
    TOKENS=$(curl -sf --unix-socket "$TOKEN_SOCKET" \
        -H "Authorization: Bearer $JWT" \
        http://localhost/tokens 2>/dev/null) || true

    if [ -n "$TOKENS" ]; then
        break
    fi

    if [ $attempt -eq $MAX_RETRIES ]; then
        echo "ERROR: Failed to exchange tokens after $MAX_RETRIES attempts"
        exit 1
    fi

    echo "Retry $attempt/$MAX_RETRIES in ${RETRY_DELAY}s..."
    sleep $RETRY_DELAY
done

# 4. Export tokens as environment variables
export ANTHROPIC_API_KEY=$(echo "$TOKENS" | jq -r '.ANTHROPIC_API_KEY // empty')
export GITHUB_TOKEN=$(echo "$TOKENS" | jq -r '.GITHUB_TOKEN // empty')

if [ -z "$ANTHROPIC_API_KEY" ]; then
    echo "ERROR: ANTHROPIC_API_KEY not received (expected for agent)"
    exit 1
fi

if [ -z "$GITHUB_TOKEN" ]; then
    echo "ERROR: GITHUB_TOKEN not received"
    exit 1
fi

echo "Tokens acquired successfully"

# 5. Setup git
cd /work
git config user.email "flashpods@local"
git config user.name "Flashpods Agent"

if [ -n "$FLASHPODS_GIT_BRANCH" ]; then
    git checkout -b "$FLASHPODS_GIT_BRANCH" 2>/dev/null || git checkout "$FLASHPODS_GIT_BRANCH"
fi

# 6. Run Claude with task
echo "Starting Claude agent..."
claude --print --dangerously-skip-permissions \
    --task "$FLASHPODS_TASK" \
    --context "$FLASHPODS_CONTEXT" \
    /work

# 7. Push changes and create artifacts
if [ -n "$FLASHPODS_GIT_BRANCH" ]; then
    git add -A
    git commit -m "Flashpods agent: $FLASHPODS_TASK" || true
    git push origin "$FLASHPODS_GIT_BRANCH" || echo "WARNING: git push failed"
    git diff main > /artifacts/changes.patch 2>/dev/null || true
fi

echo "Task completed. See branch: ${FLASHPODS_GIT_BRANCH:-main}" > /artifacts/summary.md

echo "Agent completed successfully"
```

## Container Entrypoint (Worker)

Workers have a simpler entrypoint since they don't need ANTHROPIC_API_KEY:

```bash
#!/bin/bash
set -e

# Workers may optionally fetch GITHUB_TOKEN for git operations
# If not needed, can skip token exchange entirely

if [ "${FLASHPODS_NEED_GITHUB_TOKEN:-false}" = "true" ]; then
    # Same token exchange as agent, but expects null ANTHROPIC_API_KEY
    # ... (similar to agent entrypoint)
fi

# Execute the command directly
exec "$@"
```

## Socket Permissions

| Path | Owner | Group | Mode |
|------|-------|-------|------|
| /run/flashpods/token.sock | flashpods | flashpods | 0660 |

## Secret Storage

**Location:** `/etc/flashpods/secrets.json`

**Format:**
```json
{
  "anthropic_api_key": "sk-ant-...",
  "github_token": "ghp_..."
}
```

**Permissions:** `0600 root:root` (readable only by root/token-service)

**Token service must run as root** or have appropriate capabilities to read secrets file.

## Startup Validation

Token service performs these checks on startup:
1. Secrets file exists and is readable
2. Secrets file contains valid JSON
3. Required keys (anthropic_api_key, github_token) are present
4. SPIRE trust bundle is loadable

If any check fails, token service exits with error (fatal configuration error).

## Monitoring

Token service logs:
- Each token exchange request (SPIFFE ID, success/failure)
- JWT validation failures (for security auditing)
- Secrets file reload events

## Related Specs

- [SPIRE](./08-spire.md) - Identity attestation
- [Trust Boundary](./02-trust-boundary.md) - Security model
- [Container Mounts](./15-container-mounts.md) - Socket mounts
- [Validation Tests](./14-validation.md) - Full test suite
- [NixOS Configuration](./13-nixos.md) - Service setup
