use crate::snapshot::TaskView;

/// What a request can end in besides success. The HTTP layer maps each to one status code.
#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Refused(String),
    /// The command's `expected_version` was out of date; carries the task as stored now.
    Stale(Box<TaskView>),
    Internal(String),
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Internal(format!("database error: {e}"))
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::NotFound(m) | AppError::Refused(m) | AppError::Internal(m) => f.write_str(m),
            AppError::Stale(t) => write!(f, "task {} is at version {}", t.id, t.version),
        }
    }
}

impl std::error::Error for AppError {}
