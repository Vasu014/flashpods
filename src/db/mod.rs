pub use pool::DbPool;

mod pool;

pub type Database = DbPool;

pub async fn init_db(db_path: &str) -> Result<Database, sqlx::Error> {
    let db = Database::new(db_path).await?;
    
    pool::run_migrations(&db).await?;
    
    Ok(db)
}
