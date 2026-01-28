use sqlx::{SqlitePool, Row};
use tracing::info;

#[derive(Clone)]
pub struct DbPool(SqlitePool);

impl DbPool {
    pub async fn new(db_path: &str) -> Result<Self, sqlx::Error> {
        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path)).await?;
        Ok(Self(pool))
    }

    pub fn inner(&self) -> &SqlitePool {
        &self.0
    }
}

pub async fn run_migrations(pool: &DbPool) -> Result<(), sqlx::Error> {
    info!("Running database migrations");
    
    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL DEFAULT 'default',
            job_type TEXT NOT NULL,
            status TEXT NOT NULL,
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
    "#).execute(pool.inner()).await?;

    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS idempotency_keys (
            client_job_id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES jobs(id),
            active INTEGER NOT NULL DEFAULT 1
        )
    "#).execute(pool.inner()).await?;

    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS uploads (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL DEFAULT 'default',
            state TEXT NOT NULL,
            size_bytes INTEGER,
            file_count INTEGER,
            created_at TEXT NOT NULL,
            finalized_at TEXT,
            consumed_at TEXT,
            expires_at TEXT,
            job_id TEXT REFERENCES jobs(id)
        )
    "#).execute(pool.inner()).await?;

    sqlx::query(r#"
        CREATE TABLE IF NOT EXISTS artifacts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id TEXT NOT NULL REFERENCES jobs(id),
            name TEXT NOT NULL,
            path TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(job_id, name)
        )
    "#).execute(pool.inner()).await?;

    info!("Database migrations completed");
    Ok(())
}
