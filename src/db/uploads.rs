use crate::models::{Upload, UploadState};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tracing::info;

pub struct UploadRepository {
    pool: SqlitePool,
}

impl UploadRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Get an upload by ID
    pub async fn get(&self, id: &str) -> Result<Option<Upload>, sqlx::Error> {
        let row = sqlx::query_as::<_, UploadRow>(
            "SELECT id, user_id, state, size_bytes, file_count, created_at, finalized_at, consumed_at, expires_at, job_id FROM uploads WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into_upload()))
    }

    /// Create a new upload (called when rsync starts creating files)
    pub async fn create(&self, id: &str, user_id: &str) -> Result<Upload, sqlx::Error> {
        let now = Utc::now();
        let state = "uploading";

        sqlx::query(
            "INSERT INTO uploads (id, user_id, state, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(user_id)
        .bind(state)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        // Set initial expiry for uploading state (30 min default)
        let expires_at = now + chrono::Duration::minutes(30);
        sqlx::query(
            "UPDATE uploads SET expires_at = ? WHERE id = ?",
        )
        .bind(expires_at.to_rfc3339())
        .bind(id)
        .execute(&self.pool)
        .await?;

        self.get(id).await?.ok_or_else(|| {
            sqlx::Error::RowNotFound
        })
    }

    /// Finalize an upload - transition from uploading to finalized
    pub async fn finalize(&self, id: &str, size_bytes: i64, file_count: i64) -> Result<Upload, FinalizeError> {
        let upload = self.get(id).await?.ok_or(FinalizeError::NotFound)?;

        match upload.state {
            UploadState::Uploading => {
                let now = Utc::now();
                let expires_at = now + chrono::Duration::minutes(60);

                sqlx::query(
                    r#"UPDATE uploads
                       SET state = 'finalized',
                           size_bytes = ?,
                           file_count = ?,
                           finalized_at = ?,
                           expires_at = ?
                       WHERE id = ?"#,
                )
                .bind(size_bytes)
                .bind(file_count)
                .bind(now.to_rfc3339())
                .bind(expires_at.to_rfc3339())
                .bind(id)
                .execute(&self.pool)
                .await?;

                info!("Finalized upload {} ({} bytes, {} files)", id, size_bytes, file_count);
                self.get(id).await?.ok_or(FinalizeError::NotFound)
            }
            UploadState::Finalized => Err(FinalizeError::AlreadyFinalized),
            UploadState::Consumed => Err(FinalizeError::AlreadyConsumed),
            UploadState::Expired => Err(FinalizeError::Expired),
        }
    }

    /// Mark upload as consumed (called when job reaches running state)
    pub async fn consume(&self, id: &str, job_id: &str) -> Result<(), sqlx::Error> {
        let now = Utc::now();
        sqlx::query(
            r#"UPDATE uploads
               SET state = 'consumed',
                   consumed_at = ?,
                   job_id = ?
               WHERE id = ?"#,
        )
        .bind(now.to_rfc3339())
        .bind(job_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        info!("Consumed upload {} for job {}", id, job_id);
        Ok(())
    }

    /// Delete an upload (soft delete by marking as expired)
    pub async fn delete(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE uploads SET state = 'expired' WHERE id = ? AND state IN ('uploading', 'finalized')",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;

        let deleted = result.rows_affected() > 0;
        if deleted {
            info!("Deleted upload {}", id);
        }
        Ok(deleted)
    }

    /// Get total disk usage for uploads in uploading or finalized state
    pub async fn get_total_disk_usage(&self) -> Result<i64, sqlx::Error> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM uploads WHERE state IN ('uploading', 'finalized')",
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(v,)| v).unwrap_or(0))
    }

    /// Get expired uploads for cleanup
    pub async fn get_expired(&self) -> Result<Vec<Upload>, sqlx::Error> {
        let now = Utc::now();
        let rows = sqlx::query_as::<_, UploadRow>(
            "SELECT id, user_id, state, size_bytes, file_count, created_at, finalized_at, consumed_at, expires_at, job_id
             FROM uploads
             WHERE expires_at < ? AND state IN ('uploading', 'finalized')",
        )
        .bind(now.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_upload()).collect())
    }

    /// Mark upload as expired
    pub async fn mark_expired(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE uploads SET state = 'expired' WHERE id = ?",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FinalizeError {
    #[error("Upload not found")]
    NotFound,
    #[error("Upload already finalized")]
    AlreadyFinalized,
    #[error("Upload already consumed")]
    AlreadyConsumed,
    #[error("Upload expired")]
    Expired,
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Raw database row for uploads
#[derive(sqlx::FromRow)]
struct UploadRow {
    id: String,
    user_id: String,
    state: String,
    size_bytes: Option<i64>,
    file_count: Option<i64>,
    created_at: String,
    finalized_at: Option<String>,
    consumed_at: Option<String>,
    expires_at: Option<String>,
    job_id: Option<String>,
}

impl UploadRow {
    fn into_upload(self) -> Upload {
        Upload {
            id: self.id,
            user_id: self.user_id,
            state: self.state.parse().unwrap_or(UploadState::Uploading),
            size_bytes: self.size_bytes,
            file_count: self.file_count,
            created_at: parse_datetime(&self.created_at),
            finalized_at: self.finalized_at.and_then(|s| parse_datetime_opt(&s)),
            consumed_at: self.consumed_at.and_then(|s| parse_datetime_opt(&s)),
            expires_at: self.expires_at.and_then(|s| parse_datetime_opt(&s)),
            job_id: self.job_id,
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
    use sqlx::SqlitePool;

    async fn create_test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

        // Create tables
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

        pool
    }

    #[tokio::test]
    async fn test_create_upload() {
        let pool = create_test_pool().await;
        let repo = UploadRepository::new(pool);

        let upload = repo.create("upload_test1", "user1").await.unwrap();
        assert_eq!(upload.id, "upload_test1");
        assert_eq!(upload.state, UploadState::Uploading);
        assert!(upload.expires_at.is_some());
    }

    #[tokio::test]
    async fn test_finalize_upload() {
        let pool = create_test_pool().await;
        let repo = UploadRepository::new(pool);

        repo.create("upload_test2", "user1").await.unwrap();
        let upload = repo.finalize("upload_test2", 1024, 5).await.unwrap();

        assert_eq!(upload.state, UploadState::Finalized);
        assert_eq!(upload.size_bytes, Some(1024));
        assert_eq!(upload.file_count, Some(5));
        assert!(upload.finalized_at.is_some());
    }

    #[tokio::test]
    async fn test_finalize_already_finalized() {
        let pool = create_test_pool().await;
        let repo = UploadRepository::new(pool);

        repo.create("upload_test3", "user1").await.unwrap();
        repo.finalize("upload_test3", 1024, 5).await.unwrap();

        let result = repo.finalize("upload_test3", 2048, 10).await;
        assert!(matches!(result, Err(FinalizeError::AlreadyFinalized)));
    }

    #[tokio::test]
    async fn test_finalize_not_found() {
        let pool = create_test_pool().await;
        let repo = UploadRepository::new(pool);

        let result = repo.finalize("nonexistent", 1024, 5).await;
        assert!(matches!(result, Err(FinalizeError::NotFound)));
    }

    #[tokio::test]
    async fn test_consume_upload() {
        let pool = create_test_pool().await;
        let repo = UploadRepository::new(pool);

        repo.create("upload_test4", "user1").await.unwrap();
        repo.finalize("upload_test4", 1024, 5).await.unwrap();
        repo.consume("upload_test4", "job_123").await.unwrap();

        let upload = repo.get("upload_test4").await.unwrap().unwrap();
        assert_eq!(upload.state, UploadState::Consumed);
        assert_eq!(upload.job_id, Some("job_123".to_string()));
    }

    #[tokio::test]
    async fn test_delete_upload() {
        let pool = create_test_pool().await;
        let repo = UploadRepository::new(pool);

        repo.create("upload_test5", "user1").await.unwrap();
        let deleted = repo.delete("upload_test5").await.unwrap();
        assert!(deleted);

        let upload = repo.get("upload_test5").await.unwrap().unwrap();
        assert_eq!(upload.state, UploadState::Expired);
    }

    #[tokio::test]
    async fn test_get_total_disk_usage() {
        let pool = create_test_pool().await;
        let repo = UploadRepository::new(pool);

        repo.create("upload_test6", "user1").await.unwrap();
        repo.create("upload_test7", "user1").await.unwrap();

        repo.finalize("upload_test6", 1000, 1).await.unwrap();
        repo.finalize("upload_test7", 2000, 2).await.unwrap();

        let usage = repo.get_total_disk_usage().await.unwrap();
        assert_eq!(usage, 3000);
    }
}
