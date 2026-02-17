use sqlx::SqlitePool;
use tracing::info;

#[derive(Clone)]
pub struct DbPool(SqlitePool);

impl DbPool {
    pub async fn new(db_path: &str) -> Result<Self, sqlx::Error> {
        let pool = SqlitePool::connect(&format!("sqlite:{}?mode=rwc", db_path)).await?;
        Ok(Self(pool))
    }

    pub fn inner(&self) -> &SqlitePool {
        &self.0
    }
}

pub async fn run_migrations(pool: &DbPool) -> Result<(), sqlx::Error> {
    info!("Running database migrations");

    // Create jobs table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS jobs (
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
    .execute(pool.inner())
    .await?;

    // Create jobs indexes
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_jobs_user_id ON jobs(user_id)")
        .execute(pool.inner())
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status)")
        .execute(pool.inner())
        .await?;

    // Create idempotency_keys table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS idempotency_keys (
            client_job_id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
            active INTEGER NOT NULL DEFAULT 1
        )
    "#,
    )
    .execute(pool.inner())
    .await?;

    // Create idempotency_keys index for active keys
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_idempotency_active ON idempotency_keys(client_job_id) WHERE active = 1",
    )
    .execute(pool.inner())
    .await?;

    // Create uploads table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS uploads (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL DEFAULT 'default',
            state TEXT NOT NULL CHECK (state IN ('uploading', 'finalized', 'consumed', 'expired')),
            size_bytes INTEGER,
            file_count INTEGER,
            created_at TEXT NOT NULL,
            finalized_at TEXT,
            consumed_at TEXT,
            expires_at TEXT,
            job_id TEXT REFERENCES jobs(id) ON DELETE SET NULL
        )
    "#,
    )
    .execute(pool.inner())
    .await?;

    // Create uploads indexes
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_uploads_state ON uploads(state)")
        .execute(pool.inner())
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_uploads_expires_at ON uploads(expires_at)")
        .execute(pool.inner())
        .await?;

    // Create artifacts table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS artifacts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            path TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(job_id, name)
        )
    "#,
    )
    .execute(pool.inner())
    .await?;

    // Create artifacts index
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_artifacts_job_id ON artifacts(job_id)")
        .execute(pool.inner())
        .await?;

    info!("Database migrations completed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Row, sqlite::SqliteRow};

    async fn create_test_pool() -> DbPool {
        // Use in-memory database for tests
        let pool = DbPool::new(":memory:").await.expect("Failed to create test pool");
        run_migrations(&pool).await.expect("Failed to run migrations");
        pool
    }

    #[tokio::test]
    async fn test_schema_creates_all_tables() {
        let pool = create_test_pool().await;

        // Verify all tables exist
        let tables: Vec<String> = sqlx::query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map(|row: SqliteRow| row.get("name"))
        .fetch_all(pool.inner())
        .await
        .expect("Failed to query tables");

        assert_eq!(tables, vec!["artifacts", "idempotency_keys", "jobs", "uploads"]);
    }

    #[tokio::test]
    async fn test_schema_creates_all_indexes() {
        let pool = create_test_pool().await;

        let indexes: Vec<String> = sqlx::query(
            "SELECT name FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map(|row: SqliteRow| row.get("name"))
        .fetch_all(pool.inner())
        .await
        .expect("Failed to query indexes");

        let expected = vec![
            "idx_artifacts_job_id",
            "idx_idempotency_active",
            "idx_jobs_status",
            "idx_jobs_user_id",
            "idx_uploads_expires_at",
            "idx_uploads_state",
        ];

        for expected_idx in expected {
            assert!(indexes.contains(&expected_idx.to_string()), "Missing index: {}", expected_idx);
        }
    }

    #[tokio::test]
    async fn test_jobs_table_constraints() {
        let pool = create_test_pool().await;

        // Test valid job insert
        let result = sqlx::query(
            r#"
            INSERT INTO jobs (id, job_type, status, image, created_at)
            VALUES ('job_test1', 'worker', 'pending', 'rust:latest', '2026-01-28T10:00:00Z')
            "#,
        )
        .execute(pool.inner())
        .await;

        assert!(result.is_ok(), "Valid job insert should succeed");

        // Test invalid job_type
        let result = sqlx::query(
            r#"
            INSERT INTO jobs (id, job_type, status, image, created_at)
            VALUES ('job_test2', 'invalid_type', 'pending', 'rust:latest', '2026-01-28T10:00:00Z')
            "#,
        )
        .execute(pool.inner())
        .await;

        assert!(result.is_err(), "Invalid job_type should fail CHECK constraint");

        // Test invalid status
        let result = sqlx::query(
            r#"
            INSERT INTO jobs (id, job_type, status, image, created_at)
            VALUES ('job_test3', 'worker', 'invalid_status', 'rust:latest', '2026-01-28T10:00:00Z')
            "#,
        )
        .execute(pool.inner())
        .await;

        assert!(result.is_err(), "Invalid status should fail CHECK constraint");
    }

    #[tokio::test]
    async fn test_uploads_table_constraints() {
        let pool = create_test_pool().await;

        // Test valid upload insert
        let result = sqlx::query(
            r#"
            INSERT INTO uploads (id, state, created_at)
            VALUES ('upload_test1', 'uploading', '2026-01-28T10:00:00Z')
            "#,
        )
        .execute(pool.inner())
        .await;

        assert!(result.is_ok(), "Valid upload insert should succeed");

        // Test invalid state
        let result = sqlx::query(
            r#"
            INSERT INTO uploads (id, state, created_at)
            VALUES ('upload_test2', 'invalid_state', '2026-01-28T10:00:00Z')
            "#,
        )
        .execute(pool.inner())
        .await;

        assert!(result.is_err(), "Invalid state should fail CHECK constraint");
    }

    #[tokio::test]
    async fn test_idempotency_keys_foreign_key() {
        let pool = create_test_pool().await;

        // Insert a job first
        sqlx::query(
            r#"
            INSERT INTO jobs (id, job_type, status, image, created_at)
            VALUES ('job_fk_test', 'worker', 'pending', 'rust:latest', '2026-01-28T10:00:00Z')
            "#,
        )
        .execute(pool.inner())
        .await
        .expect("Failed to insert job");

        // Test valid idempotency key insert
        let result = sqlx::query(
            r#"
            INSERT INTO idempotency_keys (client_job_id, job_id, active)
            VALUES ('client-123', 'job_fk_test', 1)
            "#,
        )
        .execute(pool.inner())
        .await;

        assert!(result.is_ok(), "Valid idempotency key insert should succeed");

        // Test foreign key constraint - should fail for non-existent job
        let result = sqlx::query(
            r#"
            INSERT INTO idempotency_keys (client_job_id, job_id, active)
            VALUES ('client-456', 'nonexistent_job', 1)
            "#,
        )
        .execute(pool.inner())
        .await;

        assert!(result.is_err(), "Foreign key constraint should fail for non-existent job");
    }

    #[tokio::test]
    async fn test_artifacts_unique_constraint() {
        let pool = create_test_pool().await;

        // Insert a job first
        sqlx::query(
            r#"
            INSERT INTO jobs (id, job_type, status, image, created_at)
            VALUES ('job_artifact_test', 'worker', 'completed', 'rust:latest', '2026-01-28T10:00:00Z')
            "#,
        )
        .execute(pool.inner())
        .await
        .expect("Failed to insert job");

        // Insert first artifact
        let result = sqlx::query(
            r#"
            INSERT INTO artifacts (job_id, name, path, size_bytes, created_at)
            VALUES ('job_artifact_test', 'myapp', '/artifacts/myapp', 1024, '2026-01-28T10:05:00Z')
            "#,
        )
        .execute(pool.inner())
        .await;

        assert!(result.is_ok(), "First artifact insert should succeed");

        // Try to insert duplicate artifact name for same job
        let result = sqlx::query(
            r#"
            INSERT INTO artifacts (job_id, name, path, size_bytes, created_at)
            VALUES ('job_artifact_test', 'myapp', '/artifacts/myapp2', 2048, '2026-01-28T10:06:00Z')
            "#,
        )
        .execute(pool.inner())
        .await;

        assert!(result.is_err(), "Duplicate artifact name should fail UNIQUE constraint");
    }

    #[tokio::test]
    async fn test_jobs_default_values() {
        let pool = create_test_pool().await;

        // Insert job with minimal fields
        sqlx::query(
            r#"
            INSERT INTO jobs (id, job_type, status, image, created_at)
            VALUES ('job_defaults', 'worker', 'pending', 'rust:latest', '2026-01-28T10:00:00Z')
            "#,
        )
        .execute(pool.inner())
        .await
        .expect("Failed to insert job");

        // Verify default values
        let row = sqlx::query("SELECT user_id, cpus, memory_gb, timeout_minutes FROM jobs WHERE id = 'job_defaults'")
            .fetch_one(pool.inner())
            .await
            .expect("Failed to fetch job");

        assert_eq!(row.get::<String, _>("user_id"), "default");
        assert_eq!(row.get::<i32, _>("cpus"), 2);
        assert_eq!(row.get::<i32, _>("memory_gb"), 4);
        assert_eq!(row.get::<i32, _>("timeout_minutes"), 30);
    }

    #[tokio::test]
    async fn test_uploads_default_values() {
        let pool = create_test_pool().await;

        // Insert upload with minimal fields
        sqlx::query(
            r#"
            INSERT INTO uploads (id, state, created_at)
            VALUES ('upload_defaults', 'uploading', '2026-01-28T10:00:00Z')
            "#,
        )
        .execute(pool.inner())
        .await
        .expect("Failed to insert upload");

        // Verify default values
        let row = sqlx::query("SELECT user_id FROM uploads WHERE id = 'upload_defaults'")
            .fetch_one(pool.inner())
            .await
            .expect("Failed to fetch upload");

        assert_eq!(row.get::<String, _>("user_id"), "default");
    }

    #[tokio::test]
    async fn test_idempotency_active_index_works() {
        let pool = create_test_pool().await;

        // Insert a job
        sqlx::query(
            r#"
            INSERT INTO jobs (id, job_type, status, image, created_at)
            VALUES ('job_idem_test', 'worker', 'pending', 'rust:latest', '2026-01-28T10:00:00Z')
            "#,
        )
        .execute(pool.inner())
        .await
        .expect("Failed to insert job");

        // Insert active idempotency key
        sqlx::query(
            r#"
            INSERT INTO idempotency_keys (client_job_id, job_id, active)
            VALUES ('active-key-1', 'job_idem_test', 1)
            "#,
        )
        .execute(pool.inner())
        .await
        .expect("Failed to insert active key");

        // Insert inactive idempotency key
        sqlx::query(
            r#"
            INSERT INTO idempotency_keys (client_job_id, job_id, active)
            VALUES ('inactive-key-1', 'job_idem_test', 0)
            "#,
        )
        .execute(pool.inner())
        .await
        .expect("Failed to insert inactive key");

        // Query should only return active keys due to partial index
        let rows: Vec<String> = sqlx::query(
            "SELECT client_job_id FROM idempotency_keys WHERE active = 1",
        )
        .map(|row: SqliteRow| row.get(0))
        .fetch_all(pool.inner())
        .await
        .expect("Failed to query active keys");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], "active-key-1");
    }

    #[tokio::test]
    async fn test_resource_usage_query() {
        let pool = create_test_pool().await;

        // Insert some running jobs
        sqlx::query(
            r#"
            INSERT INTO jobs (id, job_type, status, image, cpus, memory_gb, created_at)
            VALUES 
                ('job_res1', 'worker', 'running', 'rust:latest', 4, 8, '2026-01-28T10:00:00Z'),
                ('job_res2', 'worker', 'starting', 'rust:latest', 2, 4, '2026-01-28T10:01:00Z'),
                ('job_res3', 'worker', 'completed', 'rust:latest', 4, 8, '2026-01-28T10:02:00Z')
            "#,
        )
        .execute(pool.inner())
        .await
        .expect("Failed to insert jobs");

        // Query resource usage
        let row = sqlx::query(
            r#"
            SELECT
              SUM(cpus) as used_cpus,
              SUM(memory_gb) as used_memory_gb,
              COUNT(*) as running_jobs
            FROM jobs
            WHERE status IN ('starting', 'running')
            "#,
        )
        .fetch_one(pool.inner())
        .await
        .expect("Failed to query resource usage");

        assert_eq!(row.get::<Option<i32>, _>("used_cpus"), Some(6));
        assert_eq!(row.get::<Option<i32>, _>("used_memory_gb"), Some(12));
        assert_eq!(row.get::<i32, _>("running_jobs"), 2);
    }

    #[tokio::test]
    async fn test_expired_uploads_query() {
        let pool = create_test_pool().await;

        // Insert uploads with different states
        sqlx::query(
            r#"
            INSERT INTO uploads (id, state, created_at, expires_at)
            VALUES 
                ('upload_exp1', 'uploading', '2026-01-28T09:00:00Z', '2026-01-28T09:30:00Z'),
                ('upload_exp2', 'finalized', '2026-01-28T09:00:00Z', '2026-01-28T10:00:00Z'),
                ('upload_exp3', 'consumed', '2026-01-28T09:00:00Z', '2026-01-28T09:30:00Z')
            "#,
        )
        .execute(pool.inner())
        .await
        .expect("Failed to insert uploads");

        // Query expired uploads (uploading or finalized, past expires_at)
        let rows: Vec<String> = sqlx::query(
            r#"
            SELECT id FROM uploads
            WHERE expires_at < '2026-01-28T10:30:00Z'
              AND state IN ('uploading', 'finalized')
            "#,
        )
        .map(|row: SqliteRow| row.get(0))
        .fetch_all(pool.inner())
        .await
        .expect("Failed to query expired uploads");

        assert_eq!(rows.len(), 2);
        assert!(rows.contains(&"upload_exp1".to_string()));
        assert!(rows.contains(&"upload_exp2".to_string()));
    }

    #[tokio::test]
    async fn test_migrations_are_idempotent() {
        let pool = create_test_pool().await;

        // Run migrations again on the same pool
        let result = run_migrations(&pool).await;

        // Should succeed without error (IF NOT EXISTS)
        assert!(result.is_ok(), "Migrations should be idempotent");
    }
}
