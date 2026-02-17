use crate::models::{Job, JobStatus, JobType};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tracing::info;
use uuid::Uuid;

pub struct JobRepository {
    pool: SqlitePool,
}

impl JobRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Generate a new job ID
    pub fn generate_id() -> String {
        format!("job_{}", &Uuid::new_v4().to_string().replace("-", "")[..12])
    }

    /// Get a job by ID
    pub async fn get(&self, id: &str) -> Result<Option<Job>, sqlx::Error> {
        let row = sqlx::query_as::<_, JobRow>(
            "SELECT id, user_id, job_type, status, command, task, context, git_branch,
                    files_id, image, cpus, memory_gb, timeout_minutes, container_id,
                    exit_code, error, created_at, started_at, completed_at
             FROM jobs WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_job()))
    }

    /// Get a job by client job ID (idempotency key)
    pub async fn get_by_client_id(&self, client_job_id: &str) -> Result<Option<Job>, sqlx::Error> {
        let row = sqlx::query_as::<_, JobRow>(
            "SELECT j.id, j.user_id, j.job_type, j.status, j.command, j.task, j.context,
                    j.git_branch, j.files_id, j.image, j.cpus, j.memory_gb, j.timeout_minutes,
                    j.container_id, j.exit_code, j.error, j.created_at, j.started_at, j.completed_at
             FROM jobs j
             JOIN idempotency_keys ik ON j.id = ik.job_id
             WHERE ik.client_job_id = ? AND ik.active = 1",
        )
        .bind(client_job_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_job()))
    }

    /// Create a new job
    pub async fn create(&self, job: &Job, client_job_id: Option<&str>) -> Result<Job, sqlx::Error> {
        sqlx::query(
            "INSERT INTO jobs (id, user_id, job_type, status, command, task, context, git_branch,
                               files_id, image, cpus, memory_gb, timeout_minutes, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&job.id)
        .bind(&job.user_id)
        .bind(job.job_type.to_string())
        .bind(job.status.to_string())
        .bind(&job.command)
        .bind(&job.task)
        .bind(&job.context)
        .bind(&job.git_branch)
        .bind(&job.files_id)
        .bind(&job.image)
        .bind(job.cpus)
        .bind(job.memory_gb)
        .bind(job.timeout_minutes)
        .bind(job.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        // Create idempotency key if provided
        if let Some(cid) = client_job_id {
            sqlx::query(
                "INSERT INTO idempotency_keys (client_job_id, job_id, active) VALUES (?, ?, 1)",
            )
            .bind(cid)
            .bind(&job.id)
            .execute(&self.pool)
            .await?;
        }

        info!("Created job {} (type: {:?})", job.id, job.job_type);
        self.get(&job.id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    /// Update job status
    pub async fn update_status(&self, id: &str, status: JobStatus) -> Result<(), sqlx::Error> {
        let now = Utc::now();

        let (started_at, completed_at) = match status {
            JobStatus::Running => (Some(now.to_rfc3339()), None),
            JobStatus::Completed | JobStatus::Failed | JobStatus::TimedOut | JobStatus::Cancelled => {
                (None, Some(now.to_rfc3339()))
            }
            _ => (None, None),
        };

        if let Some(started) = started_at {
            sqlx::query(
                "UPDATE jobs SET status = ?, started_at = ? WHERE id = ?",
            )
            .bind(status.to_string())
            .bind(started)
            .bind(id)
            .execute(&self.pool)
            .await?;
        } else if let Some(completed) = completed_at {
            sqlx::query(
                "UPDATE jobs SET status = ?, completed_at = ? WHERE id = ?",
            )
            .bind(status.to_string())
            .bind(completed)
            .bind(id)
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE jobs SET status = ? WHERE id = ?",
            )
            .bind(status.to_string())
            .bind(id)
            .execute(&self.pool)
            .await?;
        }

        info!("Updated job {} status to {:?}", id, status);
        Ok(())
    }

    /// Set container ID for a job
    pub async fn set_container_id(&self, id: &str, container_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE jobs SET container_id = ? WHERE id = ?",
        )
        .bind(container_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Set exit code for a job
    pub async fn set_exit_code(&self, id: &str, exit_code: i32) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE jobs SET exit_code = ? WHERE id = ?",
        )
        .bind(exit_code)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Set error message for a job
    pub async fn set_error(&self, id: &str, error: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE jobs SET error = ? WHERE id = ?",
        )
        .bind(error)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get all jobs in starting or running state (for reconciliation)
    pub async fn get_active_jobs(&self) -> Result<Vec<Job>, sqlx::Error> {
        let rows = sqlx::query_as::<_, JobRow>(
            "SELECT id, user_id, job_type, status, command, task, context, git_branch,
                    files_id, image, cpus, memory_gb, timeout_minutes, container_id,
                    exit_code, error, created_at, started_at, completed_at
             FROM jobs WHERE status IN ('starting', 'running')",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_job()).collect())
    }

    /// Get resource usage (running jobs)
    pub async fn get_resource_usage(&self) -> Result<ResourceUsage, sqlx::Error> {
        let row: (Option<i64>, Option<i64>, i64) = sqlx::query_as(
            "SELECT SUM(cpus), SUM(memory_gb), COUNT(*) FROM jobs WHERE status IN ('starting', 'running')",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(ResourceUsage {
            used_cpus: row.0.unwrap_or(0) as i32,
            used_memory_gb: row.1.unwrap_or(0) as i32,
            running_jobs: row.2 as i32,
        })
    }

    /// List jobs with optional filters
    pub async fn list(&self, status_filter: Option<&str>, limit: i32) -> Result<Vec<Job>, sqlx::Error> {
        let rows = if let Some(filter) = status_filter {
            sqlx::query_as::<_, JobRow>(
                "SELECT id, user_id, job_type, status, command, task, context, git_branch,
                        files_id, image, cpus, memory_gb, timeout_minutes, container_id,
                        exit_code, error, created_at, started_at, completed_at
                 FROM jobs WHERE status = ?
                 ORDER BY created_at DESC LIMIT ?",
            )
            .bind(filter)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, JobRow>(
                "SELECT id, user_id, job_type, status, command, task, context, git_branch,
                        files_id, image, cpus, memory_gb, timeout_minutes, container_id,
                        exit_code, error, created_at, started_at, completed_at
                 FROM jobs
                 ORDER BY created_at DESC LIMIT ?",
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows.into_iter().map(|r| r.into_job()).collect())
    }

    /// Check if a job exists
    pub async fn exists(&self, id: &str) -> Result<bool, sqlx::Error> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM jobs WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;

        Ok(row.0 > 0)
    }
}

#[derive(Debug)]
pub struct ResourceUsage {
    pub used_cpus: i32,
    pub used_memory_gb: i32,
    pub running_jobs: i32,
}

/// Raw database row for jobs
#[derive(sqlx::FromRow)]
struct JobRow {
    id: String,
    user_id: String,
    job_type: String,
    status: String,
    command: Option<String>,
    task: Option<String>,
    context: Option<String>,
    git_branch: Option<String>,
    files_id: Option<String>,
    image: String,
    cpus: i32,
    memory_gb: i32,
    timeout_minutes: i32,
    container_id: Option<String>,
    exit_code: Option<i32>,
    error: Option<String>,
    created_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
}

impl JobRow {
    fn into_job(self) -> Job {
        Job {
            id: self.id,
            user_id: self.user_id,
            job_type: self.job_type.parse().unwrap_or(JobType::Worker),
            status: self.status.parse().unwrap_or(JobStatus::Pending),
            command: self.command,
            task: self.task,
            context: self.context,
            git_branch: self.git_branch,
            files_id: self.files_id,
            image: self.image,
            cpus: self.cpus,
            memory_gb: self.memory_gb,
            timeout_minutes: self.timeout_minutes,
            container_id: self.container_id,
            exit_code: self.exit_code,
            error: self.error,
            created_at: parse_datetime(&self.created_at),
            started_at: self.started_at.and_then(|s| parse_datetime_opt(&s)),
            completed_at: self.completed_at.and_then(|s| parse_datetime_opt(&s)),
        }
    }
}

fn parse_datetime(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn parse_datetime_opt(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn create_test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

        sqlx::query(
            r#"
            CREATE TABLE jobs (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL DEFAULT 'default',
                job_type TEXT NOT NULL CHECK (job_type IN ('worker', 'agent')),
                status TEXT NOT NULL CHECK (status IN ('pending', 'starting', 'running', 'completed', 'failed', 'timed_out', 'cancelled', 'cleaning', 'cleaned')),
                command TEXT,
                task TEXT,
                context TEXT,
                git_branch TEXT,
                files_id TEXT,
                image TEXT NOT NULL,
                cpus INTEGER NOT NULL DEFAULT 2,
                memory_gb INTEGER NOT NULL DEFAULT 4,
                timeout_minutes INTEGER NOT NULL DEFAULT 30,
                container_id TEXT,
                exit_code INTEGER,
                error TEXT,
                created_at TEXT NOT NULL,
                started_at TEXT,
                completed_at TEXT
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            r#"
            CREATE TABLE idempotency_keys (
                client_job_id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL REFERENCES jobs(id),
                active INTEGER NOT NULL DEFAULT 1
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        pool
    }

    #[tokio::test]
    async fn test_create_and_get_job() {
        let pool = create_test_pool().await;
        let repo = JobRepository::new(pool);

        let job = Job {
            id: JobRepository::generate_id(),
            user_id: "user1".to_string(),
            job_type: JobType::Worker,
            status: JobStatus::Pending,
            command: Some("echo hello".to_string()),
            task: None,
            context: None,
            git_branch: None,
            files_id: None,
            image: "ubuntu:22.04".to_string(),
            cpus: 2,
            memory_gb: 4,
            timeout_minutes: 30,
            container_id: None,
            exit_code: None,
            error: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };

        let created = repo.create(&job, None).await.unwrap();
        assert_eq!(created.id, job.id);

        let fetched = repo.get(&job.id).await.unwrap().unwrap();
        assert_eq!(fetched.job_type, JobType::Worker);
        assert_eq!(fetched.status, JobStatus::Pending);
    }

    #[tokio::test]
    async fn test_update_status() {
        let pool = create_test_pool().await;
        let repo = JobRepository::new(pool);

        let job = Job {
            id: JobRepository::generate_id(),
            user_id: "default".to_string(),
            job_type: JobType::Worker,
            status: JobStatus::Pending,
            command: Some("echo test".to_string()),
            task: None,
            context: None,
            git_branch: None,
            files_id: None,
            image: "ubuntu:22.04".to_string(),
            cpus: 2,
            memory_gb: 4,
            timeout_minutes: 30,
            container_id: None,
            exit_code: None,
            error: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };

        repo.create(&job, None).await.unwrap();

        // Update to running
        repo.update_status(&job.id, JobStatus::Running).await.unwrap();
        let updated = repo.get(&job.id).await.unwrap().unwrap();
        assert_eq!(updated.status, JobStatus::Running);
        assert!(updated.started_at.is_some());

        // Update to completed
        repo.update_status(&job.id, JobStatus::Completed).await.unwrap();
        let updated = repo.get(&job.id).await.unwrap().unwrap();
        assert_eq!(updated.status, JobStatus::Completed);
        assert!(updated.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_idempotency_key() {
        let pool = create_test_pool().await;
        let repo = JobRepository::new(pool);

        let client_job_id = "test-client-id-123";

        let job = Job {
            id: JobRepository::generate_id(),
            user_id: "default".to_string(),
            job_type: JobType::Worker,
            status: JobStatus::Pending,
            command: Some("echo test".to_string()),
            task: None,
            context: None,
            git_branch: None,
            files_id: None,
            image: "ubuntu:22.04".to_string(),
            cpus: 2,
            memory_gb: 4,
            timeout_minutes: 30,
            container_id: None,
            exit_code: None,
            error: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };

        repo.create(&job, Some(client_job_id)).await.unwrap();

        // Should find job by client ID
        let found = repo.get_by_client_id(client_job_id).await.unwrap().unwrap();
        assert_eq!(found.id, job.id);
    }

    #[tokio::test]
    async fn test_get_resource_usage() {
        let pool = create_test_pool().await;
        let repo = JobRepository::new(pool);

        // Create running job
        let job1 = Job {
            id: JobRepository::generate_id(),
            user_id: "default".to_string(),
            job_type: JobType::Worker,
            status: JobStatus::Running,
            command: Some("echo test".to_string()),
            task: None,
            context: None,
            git_branch: None,
            files_id: None,
            image: "ubuntu:22.04".to_string(),
            cpus: 4,
            memory_gb: 8,
            timeout_minutes: 30,
            container_id: None,
            exit_code: None,
            error: None,
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            completed_at: None,
        };

        let job2 = Job {
            id: JobRepository::generate_id(),
            status: JobStatus::Starting,
            cpus: 2,
            memory_gb: 4,
            ..job1.clone()
        };

        repo.create(&job1, None).await.unwrap();
        repo.create(&job2, None).await.unwrap();

        let usage = repo.get_resource_usage().await.unwrap();
        assert_eq!(usage.used_cpus, 6);
        assert_eq!(usage.used_memory_gb, 12);
        assert_eq!(usage.running_jobs, 2);
    }
}
