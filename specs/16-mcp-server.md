# MCP Server Implementation

## Overview

The MCP Server runs on the user's laptop and translates MCP tool calls into Flashpods API requests.

## Responsibilities

1. Store `FLASHPODS_API_TOKEN` securely (never shared with containers)
2. Translate MCP tool calls to HTTP API calls
3. Execute rsync for file uploads
4. Handle artifact downloads to local filesystem
5. Implement retry logic with exponential backoff
6. Handle errors gracefully and report to agent

## Configuration

**Environment variables:**

| Variable | Required | Description |
|----------|----------|-------------|
| FLASHPODS_API_TOKEN | Yes | Bearer token for API authentication |
| FLASHPODS_HOST | No | API host (default: 10.0.0.1) |
| FLASHPODS_PORT | No | API port (default: 8080) |

**Config file:** `~/.config/flashpods/config.json`

```json
{
  "api_host": "10.0.0.1",
  "api_port": 8080,
  "rsync_host": "10.0.0.1",
  "default_exclude": [".git", "node_modules", "target", "__pycache__", ".venv"],
  "retry_max_attempts": 5,
  "retry_base_delay_ms": 1000,
  "retry_max_delay_ms": 30000
}
```

## Implementation

### Core Client

```typescript
import { spawn } from 'child_process';
import * as crypto from 'crypto';

interface FlashpodsConfig {
  apiHost: string;
  apiPort: number;
  apiToken: string;
  rsyncHost: string;
  defaultExclude: string[];
  retryMaxAttempts: number;
  retryBaseDelayMs: number;
  retryMaxDelayMs: number;
}

class FlashpodsClient {
  private config: FlashpodsConfig;

  constructor(config: FlashpodsConfig) {
    this.config = config;
  }

  private get baseUrl(): string {
    return `http://${this.config.apiHost}:${this.config.apiPort}`;
  }

  private async fetch(path: string, options: RequestInit = {}): Promise<Response> {
    const url = `${this.baseUrl}${path}`;
    const headers = {
      'Authorization': `Bearer ${this.config.apiToken}`,
      'Content-Type': 'application/json',
      ...options.headers,
    };

    return fetch(url, { ...options, headers });
  }

  private async fetchWithRetry(
    path: string,
    options: RequestInit = {},
    retryOn: number[] = [429, 503]
  ): Promise<Response> {
    let lastError: Error | null = null;

    for (let attempt = 0; attempt < this.config.retryMaxAttempts; attempt++) {
      try {
        const response = await this.fetch(path, options);

        if (retryOn.includes(response.status)) {
          const delay = Math.min(
            this.config.retryBaseDelayMs * Math.pow(2, attempt),
            this.config.retryMaxDelayMs
          );
          console.log(`Retry ${attempt + 1}/${this.config.retryMaxAttempts} after ${delay}ms...`);
          await sleep(delay);
          continue;
        }

        return response;
      } catch (e) {
        lastError = e as Error;
        if (attempt < this.config.retryMaxAttempts - 1) {
          const delay = Math.min(
            this.config.retryBaseDelayMs * Math.pow(2, attempt),
            this.config.retryMaxDelayMs
          );
          await sleep(delay);
        }
      }
    }

    throw lastError || new Error('Max retries exceeded');
  }
}
```

### File Upload (rsync)

```typescript
async uploadFiles(localPath: string, exclude: string[] = []): Promise<string> {
  const uploadId = `upload_${crypto.randomUUID().slice(0, 12)}`;
  const effectiveExclude = [...this.config.defaultExclude, ...exclude];

  // 1. Execute rsync
  const args = [
    '-avz',
    '--delete',
    ...effectiveExclude.flatMap(p => ['--exclude', p]),
    `${localPath}/`,
    `rsync://${this.config.rsyncHost}/uploads/${uploadId}/`
  ];

  try {
    await this.execRsync(args);
  } catch (e) {
    if ((e as Error).message.includes('No space left')) {
      throw new FlashpodsError('upload_disk_full', 'Server disk is full');
    }
    throw new FlashpodsError('upload_failed', `rsync failed: ${(e as Error).message}`);
  }

  // 2. Finalize upload
  const response = await this.fetchWithRetry(
    `/uploads/${uploadId}/finalize`,
    { method: 'POST' }
  );

  if (!response.ok) {
    const error = await response.json();
    throw new FlashpodsError(error.error, error.message);
  }

  return uploadId;
}

private execRsync(args: string[]): Promise<void> {
  return new Promise((resolve, reject) => {
    const proc = spawn('rsync', args, { stdio: 'pipe' });

    let stderr = '';
    proc.stderr.on('data', (data) => { stderr += data.toString(); });

    proc.on('close', (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(stderr || `rsync exited with code ${code}`));
      }
    });

    proc.on('error', reject);
  });
}
```

### spawn_worker Implementation

```typescript
async spawnWorker(params: {
  command: string;
  files?: { local_path: string; exclude?: string[] };
  image?: string;
  cpus?: number;
  memory_gb?: number;
  timeout_minutes?: number;
  client_job_id?: string;
}): Promise<{ job_id: string; status: string }> {
  const clientJobId = params.client_job_id || crypto.randomUUID();

  // 1. Upload files if provided
  let filesId: string | undefined;
  if (params.files) {
    filesId = await this.uploadFiles(
      params.files.local_path,
      params.files.exclude || []
    );
  }

  // 2. Create job (with retry for 429)
  const response = await this.fetchWithRetry('/jobs', {
    method: 'POST',
    body: JSON.stringify({
      client_job_id: clientJobId,
      type: 'worker',
      command: params.command,
      files_id: filesId,
      image: params.image || 'ubuntu:22.04',
      cpus: params.cpus || 2,
      memory_gb: params.memory_gb || 4,
      timeout_minutes: params.timeout_minutes || 30,
    }),
  });

  if (!response.ok) {
    const error = await response.json();
    throw new FlashpodsError(error.error, error.message);
  }

  return response.json();
}
```

### spawn_sub_agent Implementation

```typescript
async spawnSubAgent(params: {
  task: string;
  git_branch: string;
  files?: { local_path: string; exclude?: string[] };
  context?: string;
  cpus?: number;
  memory_gb?: number;
  timeout_minutes?: number;
  client_job_id?: string;
}): Promise<{ job_id: string; status: string }> {
  const clientJobId = params.client_job_id || crypto.randomUUID();

  // 1. Upload files if provided
  let filesId: string | undefined;
  if (params.files) {
    filesId = await this.uploadFiles(
      params.files.local_path,
      params.files.exclude || []
    );
  }

  // 2. Create job
  const response = await this.fetchWithRetry('/jobs', {
    method: 'POST',
    body: JSON.stringify({
      client_job_id: clientJobId,
      type: 'agent',  // Note: API uses "agent", not "sub_agent"
      task: params.task,
      context: params.context,
      git_branch: params.git_branch,
      files_id: filesId,
      cpus: params.cpus || 2,
      memory_gb: params.memory_gb || 4,
      timeout_minutes: params.timeout_minutes || 60,
    }),
  });

  if (!response.ok) {
    const error = await response.json();
    throw new FlashpodsError(error.error, error.message);
  }

  return response.json();
}
```

### Artifact Download

```typescript
async downloadArtifact(params: {
  job_id: string;
  artifact_name: string;
  save_to?: string;
}): Promise<string> {
  const response = await this.fetch(
    `/jobs/${params.job_id}/artifacts/${encodeURIComponent(params.artifact_name)}`
  );

  if (!response.ok) {
    const error = await response.json();
    throw new FlashpodsError(error.error, error.message);
  }

  // Determine save path
  const savePath = params.save_to || `./${params.artifact_name}`;

  // Stream to file
  const fileStream = fs.createWriteStream(savePath);
  await pipeline(response.body!, fileStream);

  return savePath;
}
```

### Polling for Job Completion

```typescript
async waitForJob(
  jobId: string,
  pollIntervalMs: number = 5000,
  timeoutMs?: number
): Promise<JobStatus> {
  const startTime = Date.now();

  while (true) {
    const status = await this.getJobStatus(jobId);

    if (['completed', 'failed', 'timed_out', 'cancelled'].includes(status.status)) {
      return status;
    }

    if (timeoutMs && (Date.now() - startTime) > timeoutMs) {
      throw new FlashpodsError('poll_timeout', 'Timeout waiting for job completion');
    }

    await sleep(pollIntervalMs);
  }
}
```

## Error Handling

```typescript
class FlashpodsError extends Error {
  constructor(
    public code: string,
    message: string,
    public statusCode?: number
  ) {
    super(message);
    this.name = 'FlashpodsError';
  }

  isRetryable(): boolean {
    return [
      'insufficient_resources',
      'rate_limited',
      'service_unavailable',
    ].includes(this.code);
  }
}
```

## MCP Tool Registration

```typescript
import { Server } from '@modelcontextprotocol/sdk/server/index.js';

const server = new Server({
  name: 'flashpods',
  version: '0.1.0',
});

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
    {
      name: 'spawn_worker',
      description: '...',
      inputSchema: { ... },
    },
    {
      name: 'spawn_sub_agent',
      description: '...',
      inputSchema: { ... },
    },
    // ... other tools
  ],
}));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  try {
    switch (name) {
      case 'spawn_worker':
        return { content: [{ type: 'text', text: JSON.stringify(await client.spawnWorker(args)) }] };
      case 'spawn_sub_agent':
        return { content: [{ type: 'text', text: JSON.stringify(await client.spawnSubAgent(args)) }] };
      // ... other tools
      default:
        throw new Error(`Unknown tool: ${name}`);
    }
  } catch (e) {
    if (e instanceof FlashpodsError) {
      return {
        content: [{ type: 'text', text: JSON.stringify({ error: e.code, message: e.message }) }],
        isError: true,
      };
    }
    throw e;
  }
});
```

## Retry Strategy

| Error | Retry | Strategy |
|-------|-------|----------|
| 429 Too Many Requests | Yes | Exponential backoff, max 5 attempts |
| 503 Service Unavailable | Yes | Exponential backoff, max 5 attempts |
| 507 Insufficient Storage | No | Report to user |
| Network error | Yes | Exponential backoff, max 3 attempts |
| 4xx errors | No | Report to user |
| 5xx errors (except 503) | Yes | Linear backoff, max 2 attempts |

## Testing

```bash
# Test connectivity
curl -H "Authorization: Bearer $FLASHPODS_API_TOKEN" http://10.0.0.1:8080/health

# Test rsync
rsync rsync://10.0.0.1/uploads/

# Test MCP server
npx @anthropic-ai/claude-code --mcp-server ./path/to/flashpods-mcp
```

## Related Specs

- [MCP Tools](./10-mcp-tools.md) - Tool definitions
- [API](./11-api.md) - HTTP endpoints
- [Uploads](./05-uploads.md) - Upload flow
- [Error Codes](./17-error-codes.md) - Error handling
