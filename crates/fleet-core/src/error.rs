use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("config error: {0}")]
    Config(String),

    #[error("metrics collection failed: {0}")]
    Metrics(String),

    #[error("store error: {0}")]
    Store(#[from] rusqlite::Error),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("ping error: {0}")]
    Ping(String),
}

pub type Result<T> = std::result::Result<T, AgentError>;
