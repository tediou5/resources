#[derive(Debug, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("key not set")]
    GenIdFailure,
    #[error("crypto error: `{0}`")]
    DbExecuteFailure(String),
}

impl From<sqlx::Error> for Error {
    fn from(value: sqlx::Error) -> Self {
        Self::DbExecuteFailure(value.to_string())
    }
}
