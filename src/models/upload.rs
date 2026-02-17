use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Upload states matching database CHECK constraint
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum UploadState {
    Uploading,
    Finalized,
    Consumed,
    Expired,
}

impl std::fmt::Display for UploadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UploadState::Uploading => write!(f, "uploading"),
            UploadState::Finalized => write!(f, "finalized"),
            UploadState::Consumed => write!(f, "consumed"),
            UploadState::Expired => write!(f, "expired"),
        }
    }
}

impl std::str::FromStr for UploadState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "uploading" => Ok(UploadState::Uploading),
            "finalized" => Ok(UploadState::Finalized),
            "consumed" => Ok(UploadState::Consumed),
            "expired" => Ok(UploadState::Expired),
            _ => Err(format!("Invalid upload state: {}", s)),
        }
    }
}

/// Upload record from database
#[derive(Debug, Clone)]
pub struct Upload {
    pub id: String,
    pub user_id: String,
    pub state: UploadState,
    pub size_bytes: Option<i64>,
    pub file_count: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub finalized_at: Option<DateTime<Utc>>,
    pub consumed_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub job_id: Option<String>,
}

/// Response for upload status/finalize endpoints
#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub upload_id: String,
    pub state: UploadState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_count: Option<i64>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalized_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

impl From<Upload> for UploadResponse {
    fn from(upload: Upload) -> Self {
        Self {
            upload_id: upload.id,
            state: upload.state,
            size_bytes: upload.size_bytes,
            file_count: upload.file_count,
            created_at: upload.created_at,
            finalized_at: upload.finalized_at,
            expires_at: upload.expires_at,
        }
    }
}

/// Upload configuration
#[derive(Debug, Clone)]
pub struct UploadConfig {
    pub upload_dir: String,
    pub max_upload_size_bytes: i64,
    pub max_total_disk_bytes: i64,
    pub ttl_uploading_minutes: i32,
    pub ttl_finalized_minutes: i32,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            upload_dir: "/tmp/flashpods/uploads".to_string(),
            max_upload_size_bytes: 2 * 1024 * 1024 * 1024, // 2 GB
            max_total_disk_bytes: 10 * 1024 * 1024 * 1024, // 10 GB
            ttl_uploading_minutes: 30,
            ttl_finalized_minutes: 60,
        }
    }
}
