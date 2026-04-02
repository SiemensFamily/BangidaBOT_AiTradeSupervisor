use thiserror::Error;

#[derive(Error, Debug)]
pub enum BangidaError {
    #[error("Exchange error: {0}")]
    Exchange(String),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("REST API error: {0}")]
    RestApi(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimit(String),

    #[error("Order rejected: {0}")]
    OrderRejected(String),

    #[error("Insufficient balance: available={available}, required={required}")]
    InsufficientBalance {
        available: String,
        required: String,
    },

    #[error("Risk check failed: {0}")]
    RiskCheck(String),

    #[error("Circuit breaker triggered: {0}")]
    CircuitBreaker(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Connection lost: {0}")]
    ConnectionLost(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<serde_json::Error> for BangidaError {
    fn from(e: serde_json::Error) -> Self {
        BangidaError::Deserialization(e.to_string())
    }
}
