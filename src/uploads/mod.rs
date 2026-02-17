use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::db::FinalizeError;
use crate::models::UploadResponse;
use crate::AppState;

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/:id/finalize", axum::routing::post(finalize_upload))
        .route("/:id", axum::routing::get(get_upload).delete(delete_upload))
}

/// POST /uploads/:id/finalize
/// Mark upload as finalized after rsync completes
async fn finalize_upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let upload_dir = std::path::Path::new(&state.upload_config.upload_dir).join(&id);

    // Check if upload directory exists
    if !upload_dir.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "upload_not_found",
                "message": format!("Upload directory {} does not exist", id)
            })),
        ));
    }

    // Calculate size and file count
    let (size_bytes, file_count) = match calculate_dir_stats(&upload_dir) {
        Ok(stats) => stats,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "stat_failed",
                    "message": format!("Failed to calculate upload stats: {}", e)
                })),
            ));
        }
    };

    // Check size limit
    if size_bytes > state.upload_config.max_upload_size_bytes {
        return Err((
            StatusCode::INSUFFICIENT_STORAGE,
            Json(serde_json::json!({
                "error": "insufficient_storage",
                "message": format!(
                    "Upload size {} exceeds maximum {}",
                    size_bytes,
                    state.upload_config.max_upload_size_bytes
                )
            })),
        ));
    }

    // Check total disk usage
    match state.upload_repo.get_total_disk_usage().await {
        Ok(current_usage) => {
            if current_usage + size_bytes > state.upload_config.max_total_disk_bytes {
                return Err((
                    StatusCode::INSUFFICIENT_STORAGE,
                    Json(serde_json::json!({
                        "error": "insufficient_storage",
                        "message": "Total upload storage quota exceeded"
                    })),
                ));
            }
        }
        Err(e) => {
            tracing::error!("Failed to get disk usage: {}", e);
        }
    }

    // Create upload record if it doesn't exist (idempotent)
    if state.upload_repo.get(&id).await.ok().flatten().is_none() {
        if let Err(e) = state.upload_repo.create(&id, "default").await {
            tracing::warn!("Failed to create upload record: {}", e);
        }
    }

    // Finalize in database
    match state.upload_repo.finalize(&id, size_bytes, file_count).await {
        Ok(upload) => Ok(Json(UploadResponse::from(upload))),
        Err(e) => {
            let (status, error_code) = match e {
                FinalizeError::NotFound => (StatusCode::NOT_FOUND, "upload_not_found"),
                FinalizeError::AlreadyFinalized => {
                    (StatusCode::CONFLICT, "upload_already_finalized")
                }
                FinalizeError::AlreadyConsumed => {
                    (StatusCode::CONFLICT, "upload_already_consumed")
                }
                FinalizeError::Expired => (StatusCode::GONE, "upload_expired"),
                FinalizeError::Database(_) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, "database_error")
                }
            };
            Err((
                status,
                Json(serde_json::json!({
                    "error": error_code,
                    "message": e.to_string()
                })),
            ))
        }
    }
}

/// GET /uploads/:id
/// Get upload status
async fn get_upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.upload_repo.get(&id).await {
        Ok(Some(upload)) => Ok(Json(UploadResponse::from(upload))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "upload_not_found",
                "message": format!("Upload {} not found", id)
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

/// DELETE /uploads/:id
/// Cancel/delete an upload
async fn delete_upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Delete from filesystem first
    let upload_dir = std::path::Path::new(&state.upload_config.upload_dir).join(&id);
    if upload_dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&upload_dir) {
            tracing::warn!("Failed to delete upload directory {}: {}", id, e);
        }
    }

    // Mark as expired in database
    match state.upload_repo.delete(&id).await {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "upload_not_found",
                "message": format!("Upload {} not found or already in terminal state", id)
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

/// Calculate total size and file count for a directory
fn calculate_dir_stats(path: &std::path::Path) -> std::io::Result<(i64, i64)> {
    let mut total_size = 0i64;
    let mut file_count = 0i64;

    fn walk_dir(path: &std::path::Path, size: &mut i64, count: &mut i64) -> std::io::Result<()> {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                *size += entry.metadata()?.len() as i64;
                *count += 1;
            } else if path.is_dir() {
                walk_dir(&path, size, count)?;
            }
        }
        Ok(())
    }

    walk_dir(path, &mut total_size, &mut file_count)?;
    Ok((total_size, file_count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_dir_stats() {
        let temp_dir = tempfile::TempDir::new().unwrap();

        // Create test files
        std::fs::write(temp_dir.path().join("file1.txt"), "hello").unwrap();
        std::fs::write(temp_dir.path().join("file2.txt"), "world!").unwrap();
        std::fs::create_dir(temp_dir.path().join("subdir")).unwrap();
        std::fs::write(temp_dir.path().join("subdir/file3.txt"), "nested").unwrap();

        let (size, count) = calculate_dir_stats(temp_dir.path()).unwrap();
        assert_eq!(count, 3);
        assert_eq!(size, 5 + 6 + 6); // "hello" + "world!" + "nested"
    }

    #[test]
    fn test_calculate_dir_stats_empty() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let (size, count) = calculate_dir_stats(temp_dir.path()).unwrap();
        assert_eq!(count, 0);
        assert_eq!(size, 0);
    }

    #[tokio::test]
    async fn test_upload_repository() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();

        // Create table
        sqlx::query(
            r#"
            CREATE TABLE uploads (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL DEFAULT 'default',
                state TEXT NOT NULL CHECK (state IN ('uploading', 'finalized', 'consumed', 'expired')),
                size_bytes INTEGER,
                file_count INTEGER,
                created_at TEXT NOT NULL,
                finalized_at TEXT,
                consumed_at TEXT,
                expires_at TEXT,
                job_id TEXT
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        let repo = crate::db::UploadRepository::new(pool);

        // Test create
        let upload = repo.create("test_upload", "user1").await.unwrap();
        assert_eq!(upload.id, "test_upload");
        assert_eq!(upload.state, crate::models::UploadState::Uploading);

        // Test finalize
        let upload = repo.finalize("test_upload", 1024, 5).await.unwrap();
        assert_eq!(upload.state, crate::models::UploadState::Finalized);
        assert_eq!(upload.size_bytes, Some(1024));

        // Test get
        let upload = repo.get("test_upload").await.unwrap().unwrap();
        assert_eq!(upload.state, crate::models::UploadState::Finalized);

        // Test delete
        let deleted = repo.delete("test_upload").await.unwrap();
        assert!(deleted);

        let upload = repo.get("test_upload").await.unwrap().unwrap();
        assert_eq!(upload.state, crate::models::UploadState::Expired);
    }
}
