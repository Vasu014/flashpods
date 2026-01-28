pub fn routes() -> axum::Router {
    axum::Router::new()
        .route("/", axum::routing::get(list_artifacts))
        .route("/:name", axum::routing::get(download_artifact))
}

async fn list_artifacts() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "artifacts": [],
        "total_size_bytes": 0,
        "expires_at": "2026-01-21T11:35:00Z",
        "copy_in_progress": false
    }))
}

async fn download_artifact() -> impl axum::response::IntoResponse {
    axum::http::StatusCode::NOT_FOUND
}
