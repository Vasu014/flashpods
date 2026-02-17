use crate::AppState;

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/", axum::routing::post(create_job))
        .route("/:id", axum::routing::get(get_job).delete(kill_job))
        .route("/:id/output", axum::routing::get(get_output))
        .route("/:id/artifacts", axum::routing::get(list_artifacts))
}

async fn create_job() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "job_id": "job_placeholder",
        "status": "starting",
        "created": true
    }))
}

async fn get_job() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "id": "job_placeholder",
        "type": "worker",
        "status": "starting",
        "command": "cargo build --release",
        "image": "rust:latest",
        "cpus": 4,
        "memory_gb": 8,
        "timeout_minutes": 30,
        "created_at": "2026-01-18T10:30:00Z",
        "started_at": "2026-01-18T10:30:02Z"
    }))
}

async fn kill_job() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "job_id": "job_placeholder",
        "status": "cancelled",
        "message": "Job termination initiated"
    }))
}

async fn get_output() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "output": "",
        "lines": 0,
        "truncated": false,
        "total_bytes": 0
    }))
}

async fn list_artifacts() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "artifacts": [],
        "total_size_bytes": 0,
        "expires_at": "2026-01-21T11:35:00Z",
        "copy_in_progress": false
    }))
}
