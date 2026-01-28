pub fn routes() -> axum::Router {
    axum::Router::new()
        .route("/:id/finalize", axum::routing::post(finalize_upload))
        .route("/:id", axum::routing::get(get_upload).delete(delete_upload))
}

async fn finalize_upload() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "upload_id": "upload_placeholder",
        "state": "finalized",
        "size_bytes": 0,
        "file_count": 0,
        "finalized_at": "2026-01-21T10:30:00Z",
        "expires_at": "2026-01-21T11:00:00Z"
    }))
}

async fn get_upload() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "upload_id": "upload_placeholder",
        "state": "finalized",
        "size_bytes": 0,
        "file_count": 0,
        "created_at": "2026-01-21T10:25:00Z",
        "finalized_at": "2026-01-21T10:30:00Z",
        "expires_at": "2026-01-21T11:00:00Z"
    }))
}

async fn delete_upload() -> impl axum::response::IntoResponse {
    axum::http::StatusCode::NO_CONTENT
}
