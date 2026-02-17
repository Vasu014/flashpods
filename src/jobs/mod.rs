use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;

use crate::db::JobRepository;
use crate::models::{
    CreateJobRequest, CreateJobResponse, Job, JobResponse, JobStatus, JobType, ResourceLimits,
};
use crate::podman::{ContainerConfig, PodmanService};
use crate::AppState;

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/", axum::routing::post(create_job).get(list_jobs))
        .route("/:id", axum::routing::get(get_job).delete(kill_job))
        .route("/:id/output", axum::routing::get(get_output))
        .route("/:id/artifacts", axum::routing::get(list_artifacts))
}

/// POST /jobs - Create a new job
async fn create_job(
    State(state): State<AppState>,
    Json(req): Json<CreateJobRequest>,
) -> impl IntoResponse {
    // Parse job type
    let job_type: JobType = match req.job_type.parse() {
        Ok(t) => t,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_job_type",
                    "message": e
                })),
            ));
        }
    };

    // Validate required fields based on job type
    match job_type {
        JobType::Worker => {
            if req.command.is_none() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "missing_command",
                        "message": "Worker jobs require a 'command' field"
                    })),
                ));
            }
        }
        JobType::Agent => {
            if req.task.is_none() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "missing_task",
                        "message": "Agent jobs require a 'task' field"
                    })),
                ));
            }
        }
    }

    // Check idempotency key
    if let Some(ref client_job_id) = req.client_job_id {
        if let Ok(Some(existing_job)) = state.job_repo.get_by_client_id(client_job_id).await {
            // Return existing job if not cleaned
            if existing_job.status != JobStatus::Cleaned {
                return Ok(Json(CreateJobResponse {
                    job_id: existing_job.id,
                    status: existing_job.status,
                    created: false,
                    message: Some("Existing job returned (idempotent)".to_string()),
                }));
            }
        }
    }

    // Validate upload if files_id provided
    if let Some(ref files_id) = req.files_id {
        match state.upload_repo.get(files_id).await {
            Ok(Some(upload)) => {
                if upload.state != crate::models::UploadState::Finalized {
                    return Err((
                        StatusCode::CONFLICT,
                        Json(serde_json::json!({
                            "error": "upload_not_finalized",
                            "message": format!("Upload {} is in {} state, must be finalized", files_id, upload.state)
                        })),
                    ));
                }
            }
            Ok(None) => {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": "upload_not_found",
                        "message": format!("Upload {} not found", files_id)
                    })),
                ));
            }
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "database_error",
                        "message": e.to_string()
                    })),
                ));
            }
        }
    }

    // Clamp resource limits
    let limits = ResourceLimits::for_job_type(job_type);
    let (cpus, memory_gb, timeout_minutes) =
        limits.clamp(req.cpus, req.memory_gb, req.timeout_minutes);

    // Check resource availability
    match state.job_repo.get_resource_usage().await {
        Ok(usage) => {
            // Simple admission control: reject if adding this job would exceed limits
            // In production, you'd want configurable limits
            let max_cpus = 16;
            let max_memory_gb = 32;

            if usage.used_cpus + cpus > max_cpus {
                return Err((
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({
                        "error": "resource_exhausted",
                        "message": format!("Insufficient CPU: {} used, {} requested, {} max", usage.used_cpus, cpus, max_cpus)
                    })),
                ));
            }

            if usage.used_memory_gb + memory_gb > max_memory_gb {
                return Err((
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({
                        "error": "resource_exhausted",
                        "message": format!("Insufficient memory: {}GB used, {}GB requested, {}GB max", usage.used_memory_gb, memory_gb, max_memory_gb)
                    })),
                ));
            }
        }
        Err(e) => {
            tracing::error!("Failed to get resource usage: {}", e);
        }
    }

    // Create job record
    let job_id = JobRepository::generate_id();
    let job = Job {
        id: job_id.clone(),
        user_id: "default".to_string(),
        job_type,
        status: JobStatus::Pending,
        command: req.command.clone(),
        task: req.task.clone(),
        context: req.context.clone(),
        git_branch: req.git_branch.clone(),
        files_id: req.files_id.clone(),
        image: req.image.clone(),
        cpus,
        memory_gb,
        timeout_minutes,
        container_id: None,
        exit_code: None,
        error: None,
        created_at: Utc::now(),
        started_at: None,
        completed_at: None,
    };

    // Save to database
    let job = match state
        .job_repo
        .create(&job, req.client_job_id.as_deref())
        .await
    {
        Ok(j) => j,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "database_error",
                    "message": format!("Failed to create job: {}", e)
                })),
            ));
        }
    };

    // Start container
    // First update status to starting
    if let Err(e) = state.job_repo.update_status(&job.id, JobStatus::Starting).await {
        tracing::warn!("Failed to update status to starting: {}", e);
    }

    match start_container(&state, &job) {
        Ok(container_id) => {
            // Update job with container ID and status
            if let Err(e) = state.job_repo.set_container_id(&job.id, &container_id).await {
                tracing::error!("Failed to set container ID: {}", e);
            }
            if let Err(e) = state.job_repo.update_status(&job.id, JobStatus::Running).await {
                tracing::error!("Failed to update job status: {}", e);
            }
        }
        Err(e) => {
            tracing::error!("Failed to start container: {}", e);
            if let Err(err) = state
                .job_repo
                .update_status(&job.id, JobStatus::Failed)
                .await
            {
                tracing::error!("Failed to update job status: {}", err);
            }
            if let Err(err) = state.job_repo.set_error(&job.id, &e.to_string()).await {
                tracing::error!("Failed to set job error: {}", err);
            }
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "container_start_failed",
                    "message": e.to_string()
                })),
            ));
        }
    }

    Ok(Json(CreateJobResponse {
        job_id: job.id,
        status: JobStatus::Running,
        created: true,
        message: None,
    }))
}

/// Start a container for a job
fn start_container(state: &AppState, job: &Job) -> Result<String, crate::podman::PodmanError> {
    let config = ContainerConfig {
        job_id: job.id.clone(),
        job_type: match job.job_type {
            JobType::Worker => crate::podman::JobType::Worker,
            JobType::Agent => crate::podman::JobType::Agent,
        },
        upload_id: job.files_id.clone().unwrap_or_default(),
        image: job.image.clone(),
        command: job.command.clone(),
        cpus: job.cpus,
        memory_gb: job.memory_gb,
        task: job.task.clone(),
        context: job.context.clone(),
        git_branch: job.git_branch.clone(),
    };

    // Update status to starting
    // Note: This is a sync wrapper - the caller handles async updates
    state.podman.create_container(&config)
}

/// GET /jobs - List jobs
async fn list_jobs(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<ListJobsQuery>,
) -> impl IntoResponse {
    let status_filter = params.status.as_deref();
    let limit = params.limit.unwrap_or(20).min(100);

    match state.job_repo.list(status_filter, limit).await {
        Ok(jobs) => {
            let job_responses: Vec<JobResponse> = jobs.into_iter().map(JobResponse::from).collect();
            Ok(Json(serde_json::json!({
                "jobs": job_responses,
                "total": job_responses.len()
            })))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "database_error",
                "message": e.to_string()
            })),
        )),
    }
}

#[derive(serde::Deserialize)]
struct ListJobsQuery {
    status: Option<String>,
    limit: Option<i32>,
}

/// GET /jobs/:id - Get job details
async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.job_repo.get(&id).await {
        Ok(Some(job)) => Ok(Json(JobResponse::from(job))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "job_not_found",
                "message": format!("Job {} not found", id)
            })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "database_error",
                "message": e.to_string()
            })),
        )),
    }
}

/// DELETE /jobs/:id - Kill a job
async fn kill_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Get job
    let job = match state.job_repo.get(&id).await {
        Ok(Some(j)) => j,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "job_not_found",
                    "message": format!("Job {} not found", id)
                })),
            ));
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "database_error",
                    "message": e.to_string()
                })),
            ));
        }
    };

    // Check if job can be killed
    if job.status.is_terminal() {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "job_already_terminal",
                "message": format!("Job {} is already in terminal state: {}", id, job.status)
            })),
        ));
    }

    // Kill container
    if let Some(ref container_id) = job.container_id {
        if let Err(e) = state.podman.stop_container(container_id, 10) {
            tracing::warn!("Failed to stop container {}: {}", container_id, e);
            // Try kill as fallback
            let _ = state.podman.kill_container(container_id);
        }
    }

    // Update status
    if let Err(e) = state.job_repo.update_status(&id, JobStatus::Cancelled).await {
        tracing::error!("Failed to update job status: {}", e);
    }
    if let Err(e) = state.job_repo.set_exit_code(&id, 137).await {
        tracing::error!("Failed to set exit code: {}", e);
    }

    Ok(Json(serde_json::json!({
        "job_id": id,
        "status": "cancelled",
        "message": "Job termination initiated"
    })))
}

/// GET /jobs/:id/output - Get job output
async fn get_output(
    Path(_id): Path<String>,
) -> impl IntoResponse {
    // TODO: Implement log retrieval
    axum::Json(serde_json::json!({
        "output": "",
        "lines": 0,
        "truncated": false,
        "total_bytes": 0
    }))
}

/// GET /jobs/:id/artifacts - List job artifacts
async fn list_artifacts(
    Path(_id): Path<String>,
) -> impl IntoResponse {
    // TODO: Implement artifact listing
    axum::Json(serde_json::json!({
        "artifacts": [],
        "total_size_bytes": 0,
        "expires_at": "2026-01-21T11:35:00Z",
        "copy_in_progress": false
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_state() -> AppState {
        use std::sync::Arc;

        let rt = tokio::runtime::Runtime::new().unwrap();
        let (db, pool) = rt.block_on(async {
            let db = crate::db::init_db(":memory:").await.unwrap();
            let pool = db.inner().clone();
            (db, pool)
        });

        AppState {
            db,
            upload_repo: Arc::new(crate::db::UploadRepository::new(pool.clone())),
            job_repo: Arc::new(crate::db::JobRepository::new(pool)),
            upload_config: crate::models::UploadConfig::default(),
            podman: Arc::new(PodmanService::new()),
        }
    }

    #[test]
    fn test_resource_limits_clamp() {
        let limits = ResourceLimits::for_job_type(JobType::Worker);
        let (cpus, mem, timeout) = limits.clamp(100, 100, 200);
        assert_eq!(cpus, 8); // max for worker
        assert_eq!(mem, 16); // max for worker
        assert_eq!(timeout, 120); // max timeout

        let (cpus, mem, timeout) = limits.clamp(0, 0, 0);
        assert_eq!(cpus, 1); // min
        assert_eq!(mem, 1); // min
        assert_eq!(timeout, 1); // min

        // Values within range should be unchanged
        let (cpus, mem, timeout) = limits.clamp(4, 8, 60);
        assert_eq!(cpus, 4);
        assert_eq!(mem, 8);
        assert_eq!(timeout, 60);
    }

    #[test]
    fn test_resource_limits_agent() {
        let limits = ResourceLimits::for_job_type(JobType::Agent);
        let (cpus, mem, _) = limits.clamp(100, 100, 100);
        assert_eq!(cpus, 4); // max for agent
        assert_eq!(mem, 8); // max for agent
    }
}
