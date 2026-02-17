use std::process::Command;
use tracing::{debug, error, info, warn};

/// Container information returned by podman inspect
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub state: ContainerState,
    pub exit_code: Option<i32>,
    pub labels: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContainerState {
    Created,
    Running,
    Exited,
    Paused,
    Stopped,
    Unknown,
}

impl std::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerState::Created => write!(f, "created"),
            ContainerState::Running => write!(f, "running"),
            ContainerState::Exited => write!(f, "exited"),
            ContainerState::Paused => write!(f, "paused"),
            ContainerState::Stopped => write!(f, "stopped"),
            ContainerState::Unknown => write!(f, "unknown"),
        }
    }
}

impl std::str::FromStr for ContainerState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "created" => Ok(ContainerState::Created),
            "running" => Ok(ContainerState::Running),
            "exited" => Ok(ContainerState::Exited),
            "paused" => Ok(ContainerState::Paused),
            "stopped" => Ok(ContainerState::Stopped),
            _ => Ok(ContainerState::Unknown),
        }
    }
}

/// Job type for container configuration
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JobType {
    Worker,
    Agent,
}

impl std::fmt::Display for JobType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobType::Worker => write!(f, "worker"),
            JobType::Agent => write!(f, "agent"),
        }
    }
}

/// Container creation configuration
#[derive(Debug, Clone)]
pub struct ContainerConfig {
    pub job_id: String,
    pub job_type: JobType,
    pub upload_id: String,
    pub image: String,
    pub command: Option<String>,
    pub cpus: i32,
    pub memory_gb: i32,
    // Agent-specific fields
    pub task: Option<String>,
    pub context: Option<String>,
    pub git_branch: Option<String>,
}

/// Podman service for container lifecycle management
pub struct PodmanService {
    podman_path: String,
    upload_dir: String,
    artifacts_dir: String,
    spire_socket: String,
    token_socket: String,
}

impl PodmanService {
    pub fn new() -> Self {
        Self {
            podman_path: "podman".to_string(),
            upload_dir: "/tmp/flashpods/uploads".to_string(),
            artifacts_dir: "/var/lib/flashpods/artifacts".to_string(),
            spire_socket: "/run/spire/sockets/agent.sock".to_string(),
            token_socket: "/run/flashpods/token.sock".to_string(),
        }
    }

    pub fn with_paths(
        upload_dir: String,
        artifacts_dir: String,
        spire_socket: String,
        token_socket: String,
    ) -> Self {
        Self {
            podman_path: "podman".to_string(),
            upload_dir,
            artifacts_dir,
            spire_socket,
            token_socket,
        }
    }

    /// Create and start a container for a job
    pub fn create_container(&self, config: &ContainerConfig) -> Result<String, PodmanError> {
        let container_name = format!("job_{}", config.job_id);
        let work_mode = match config.job_type {
            JobType::Worker => "ro",
            JobType::Agent => "rw",
        };

        // Create artifacts directory
        let artifacts_path = format!("{}/{}", self.artifacts_dir, config.job_id);
        std::fs::create_dir_all(&artifacts_path)
            .map_err(|e| PodmanError::FileSystem(format!("Failed to create artifacts dir: {}", e)))?;

        let mut cmd = Command::new(&self.podman_path);
        cmd.args(["run", "-d", "--rm"]);
        cmd.args(["--name", &container_name]);
        cmd.args(["--label", "flashpods-job=true"]);
        cmd.args(["--label", &format!("flashpods-job-id={}", config.job_id)]);
        cmd.args(["--label", &format!("flashpods-job-type={}", config.job_type)]);
        cmd.args(["--cpus", &config.cpus.to_string()]);
        cmd.args(["--memory", &format!("{}g", config.memory_gb)]);
        cmd.args(["--userns=keep-id"]);
        cmd.args(["--network=slirp4netns"]);
        cmd.args(["--security-opt", "no-new-privileges"]);
        cmd.args(["--cap-drop", "ALL"]);

        // Mounts
        let work_mount = format!("{}/{}:/work:{}", self.upload_dir, config.upload_id, work_mode);
        let artifacts_mount = format!("{}:/artifacts:rw", artifacts_path);
        let spire_mount = format!("{}:/run/spire/sockets/agent.sock:ro", self.spire_socket);
        let token_mount = format!("{}:/run/flashpods/token.sock:ro", self.token_socket);

        cmd.args(["-v", &work_mount]);
        cmd.args(["-v", &artifacts_mount]);
        cmd.args(["-v", &spire_mount]);
        cmd.args(["-v", &token_mount]);

        // Environment variables for agents
        if config.job_type == JobType::Agent {
            if let Some(task) = &config.task {
                cmd.args(["-e", &format!("FLASHPODS_TASK={}", task)]);
            }
            if let Some(context) = &config.context {
                cmd.args(["-e", &format!("FLASHPODS_CONTEXT={}", context)]);
            }
            if let Some(git_branch) = &config.git_branch {
                cmd.args(["-e", &format!("FLASHPODS_GIT_BRANCH={}", git_branch)]);
            }
            cmd.args(["-e", &format!("FLASHPODS_JOB_ID={}", config.job_id)]);
        }

        // Image
        cmd.arg(&config.image);

        // Command
        match config.job_type {
            JobType::Worker => {
                if let Some(command) = &config.command {
                    cmd.args(["/bin/sh", "-c", command]);
                }
            }
            JobType::Agent => {
                cmd.arg("/entrypoint.sh");
            }
        }

        debug!("Running podman command: {:?}", cmd);

        let output = cmd.output().map_err(|e| {
            PodmanError::Command(format!("Failed to execute podman: {}", e))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Podman create failed: {}", stderr);
            return Err(PodmanError::ContainerStart(stderr.to_string()));
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        info!("Created container {} for job {}", container_id, config.job_id);

        Ok(container_id)
    }

    /// Stop a container with SIGTERM, then SIGKILL after grace period
    pub fn stop_container(&self, container_id: &str, grace_seconds: u64) -> Result<(), PodmanError> {
        info!("Stopping container {} with {}s grace period", container_id, grace_seconds);

        // First, try graceful stop with SIGTERM
        let stop_output = Command::new(&self.podman_path)
            .args(["stop", "-t", &grace_seconds.to_string(), container_id])
            .output()
            .map_err(|e| PodmanError::Command(format!("Failed to stop container: {}", e)))?;

        if stop_output.status.success() {
            info!("Container {} stopped gracefully", container_id);
            return Ok(());
        }

        // If stop failed, try kill
        warn!("Stop failed, killing container {}", container_id);
        self.kill_container(container_id)
    }

    /// Kill a container immediately with SIGKILL
    pub fn kill_container(&self, container_id: &str) -> Result<(), PodmanError> {
        info!("Killing container {}", container_id);

        let output = Command::new(&self.podman_path)
            .args(["kill", container_id])
            .output()
            .map_err(|e| PodmanError::Command(format!("Failed to kill container: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore "container not found" errors
            if !stderr.contains("no such container") {
                return Err(PodmanError::ContainerStop(stderr.to_string()));
            }
        }

        info!("Container {} killed", container_id);
        Ok(())
    }

    /// Get container information by ID or name
    pub fn inspect_container(&self, container_id: &str) -> Result<Option<ContainerInfo>, PodmanError> {
        let output = Command::new(&self.podman_path)
            .args(["inspect", "--format", "json", container_id])
            .output()
            .map_err(|e| PodmanError::Command(format!("Failed to inspect container: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no such container") || stderr.contains("not found") {
                return Ok(None);
            }
            return Err(PodmanError::ContainerInspect(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let containers: Vec<serde_json::Value> = serde_json::from_str(&stdout)
            .map_err(|e| PodmanError::Parse(format!("Failed to parse inspect output: {}", e)))?;

        if containers.is_empty() {
            return Ok(None);
        }

        let container = &containers[0];
        let state = container.get("State");

        let id = container
            .get("Id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let name = container
            .get("Name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim_start_matches('/')
            .to_string();

        let status = state
            .and_then(|s| s.get("Status"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let exit_code = state
            .and_then(|s| s.get("ExitCode"))
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);

        let labels = container
            .get("Config")
            .and_then(|c| c.get("Labels"))
            .and_then(|l| l.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        Ok(Some(ContainerInfo {
            id,
            name,
            state: status.parse().unwrap_or(ContainerState::Unknown),
            exit_code,
            labels,
        }))
    }

    /// List all flashpods containers
    pub fn list_containers(&self) -> Result<Vec<ContainerInfo>, PodmanError> {
        let output = Command::new(&self.podman_path)
            .args([
                "ps",
                "-a",
                "--filter",
                "label=flashpods-job=true",
                "--format",
                "json",
            ])
            .output()
            .map_err(|e| PodmanError::Command(format!("Failed to list containers: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PodmanError::ContainerList(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let containers: Vec<serde_json::Value> = if stdout.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&stdout)
                .map_err(|e| PodmanError::Parse(format!("Failed to parse container list: {}", e)))?
        };

        let mut result = Vec::new();
        for container in containers {
            let id = container
                .get("Id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let names = container.get("Names").and_then(|v| v.as_array());
            let name = names
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_start_matches('/')
                .to_string();

            let status = container
                .get("State")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let exit_code = container
                .get("ExitCode")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);

            let labels = container
                .get("Labels")
                .and_then(|l| l.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();

            result.push(ContainerInfo {
                id,
                name,
                state: status.parse().unwrap_or(ContainerState::Unknown),
                exit_code,
                labels,
            });
        }

        Ok(result)
    }

    /// Check if podman is available
    pub fn is_available(&self) -> bool {
        Command::new(&self.podman_path)
            .args(["--version"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Get podman version
    pub fn version(&self) -> Result<String, PodmanError> {
        let output = Command::new(&self.podman_path)
            .args(["--version"])
            .output()
            .map_err(|e| PodmanError::Command(format!("Failed to get podman version: {}", e)))?;

        if !output.status.success() {
            return Err(PodmanError::Command("Failed to get podman version".to_string()));
        }

        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(version)
    }
}

impl Default for PodmanService {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PodmanError {
    #[error("Command error: {0}")]
    Command(String),
    #[error("Failed to start container: {0}")]
    ContainerStart(String),
    #[error("Failed to stop container: {0}")]
    ContainerStop(String),
    #[error("Failed to inspect container: {0}")]
    ContainerInspect(String),
    #[error("Failed to list containers: {0}")]
    ContainerList(String),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("File system error: {0}")]
    FileSystem(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_state_display() {
        assert_eq!(ContainerState::Running.to_string(), "running");
        assert_eq!(ContainerState::Exited.to_string(), "exited");
    }

    #[test]
    fn test_container_state_from_str() {
        assert_eq!("running".parse::<ContainerState>(), Ok(ContainerState::Running));
        assert_eq!("exited".parse::<ContainerState>(), Ok(ContainerState::Exited));
        assert_eq!("unknown_state".parse::<ContainerState>(), Ok(ContainerState::Unknown));
    }

    #[test]
    fn test_job_type_display() {
        assert_eq!(JobType::Worker.to_string(), "worker");
        assert_eq!(JobType::Agent.to_string(), "agent");
    }

    #[test]
    fn test_podman_service_new() {
        let service = PodmanService::new();
        assert_eq!(service.podman_path, "podman");
        assert_eq!(service.upload_dir, "/tmp/flashpods/uploads");
    }

    #[test]
    fn test_podman_service_with_paths() {
        let service = PodmanService::with_paths(
            "/custom/uploads".to_string(),
            "/custom/artifacts".to_string(),
            "/custom/spire.sock".to_string(),
            "/custom/token.sock".to_string(),
        );
        assert_eq!(service.upload_dir, "/custom/uploads");
        assert_eq!(service.artifacts_dir, "/custom/artifacts");
    }

    // Note: Integration tests that require podman should be in a separate
    // tests/ directory with #[ignore] attribute and run with --ignored flag
}
