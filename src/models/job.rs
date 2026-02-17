use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Job type matching database schema
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
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

impl std::str::FromStr for JobType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "worker" => Ok(JobType::Worker),
            "agent" => Ok(JobType::Agent),
            _ => Err(format!("Invalid job type: {}", s)),
        }
    }
}

/// Job status matching database schema
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Starting,
    Running,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
    Cleaning,
    Cleaned,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Pending => write!(f, "pending"),
            JobStatus::Starting => write!(f, "starting"),
            JobStatus::Running => write!(f, "running"),
            JobStatus::Completed => write!(f, "completed"),
            JobStatus::Failed => write!(f, "failed"),
            JobStatus::TimedOut => write!(f, "timed_out"),
            JobStatus::Cancelled => write!(f, "cancelled"),
            JobStatus::Cleaning => write!(f, "cleaning"),
            JobStatus::Cleaned => write!(f, "cleaned"),
        }
    }
}

impl std::str::FromStr for JobStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(JobStatus::Pending),
            "starting" => Ok(JobStatus::Starting),
            "running" => Ok(JobStatus::Running),
            "completed" => Ok(JobStatus::Completed),
            "failed" => Ok(JobStatus::Failed),
            "timed_out" => Ok(JobStatus::TimedOut),
            "cancelled" => Ok(JobStatus::Cancelled),
            "cleaning" => Ok(JobStatus::Cleaning),
            "cleaned" => Ok(JobStatus::Cleaned),
            _ => Err(format!("Invalid job status: {}", s)),
        }
    }
}

impl JobStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobStatus::Completed
                | JobStatus::Failed
                | JobStatus::TimedOut
                | JobStatus::Cancelled
        )
    }
}

/// Job record from database
#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub user_id: String,
    pub job_type: JobType,
    pub status: JobStatus,
    // Worker fields
    pub command: Option<String>,
    // Agent fields
    pub task: Option<String>,
    pub context: Option<String>,
    pub git_branch: Option<String>,
    // Common fields
    pub files_id: Option<String>,
    pub image: String,
    pub cpus: i32,
    pub memory_gb: i32,
    pub timeout_minutes: i32,
    // Runtime fields
    pub container_id: Option<String>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    // Timestamps
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Request to create a new job
#[derive(Debug, Deserialize)]
pub struct CreateJobRequest {
    pub client_job_id: Option<String>,
    #[serde(rename = "type")]
    pub job_type: String,
    pub command: Option<String>,
    pub task: Option<String>,
    pub context: Option<String>,
    pub git_branch: Option<String>,
    pub files_id: Option<String>,
    #[serde(default = "default_image")]
    pub image: String,
    #[serde(default = "default_cpus")]
    pub cpus: i32,
    #[serde(default = "default_memory")]
    pub memory_gb: i32,
    #[serde(default = "default_timeout")]
    pub timeout_minutes: i32,
}

fn default_image() -> String {
    "ubuntu:22.04".to_string()
}

fn default_cpus() -> i32 {
    2
}

fn default_memory() -> i32 {
    4
}

fn default_timeout() -> i32 {
    30
}

/// Response for job creation
#[derive(Debug, Serialize)]
pub struct CreateJobResponse {
    pub job_id: String,
    pub status: JobStatus,
    pub created: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Response for job details
#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub job_type: JobType,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub image: String,
    pub cpus: i32,
    pub memory_gb: i32,
    pub timeout_minutes: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i64>,
}

impl From<Job> for JobResponse {
    fn from(job: Job) -> Self {
        let now = Utc::now();
        let elapsed_seconds = job.started_at.map(|started| (now - started).num_seconds());
        let duration_seconds = job.started_at.and_then(|started| {
            job.completed_at.map(|completed| (completed - started).num_seconds())
        });

        Self {
            id: job.id,
            job_type: job.job_type,
            status: job.status,
            command: job.command,
            task: job.task,
            image: job.image,
            cpus: job.cpus,
            memory_gb: job.memory_gb,
            timeout_minutes: job.timeout_minutes,
            exit_code: job.exit_code,
            error: job.error,
            created_at: job.created_at,
            started_at: job.started_at,
            completed_at: job.completed_at,
            elapsed_seconds,
            duration_seconds,
        }
    }
}

/// Job resource limits
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub max_cpus: i32,
    pub max_memory_gb: i32,
    pub max_timeout_minutes: i32,
}

impl ResourceLimits {
    pub fn for_job_type(job_type: JobType) -> Self {
        match job_type {
            JobType::Worker => Self {
                max_cpus: 8,
                max_memory_gb: 16,
                max_timeout_minutes: 120,
            },
            JobType::Agent => Self {
                max_cpus: 4,
                max_memory_gb: 8,
                max_timeout_minutes: 120,
            },
        }
    }

    /// Clamp values to limits
    pub fn clamp(&self, cpus: i32, memory_gb: i32, timeout_minutes: i32) -> (i32, i32, i32) {
        (
            cpus.clamp(1, self.max_cpus),
            memory_gb.clamp(1, self.max_memory_gb),
            timeout_minutes.clamp(1, self.max_timeout_minutes),
        )
    }
}
