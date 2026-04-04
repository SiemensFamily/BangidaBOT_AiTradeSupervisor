use thiserror::Error;

#[derive(Error, Debug)]
pub enum ScalperError {
    #[error("Exchange error: {0}")]
    Exchange(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Strategy error: {0}")]
    Strategy(String),

    #[error("Risk violation: {0}")]
    RiskViolation(String),

    #[error("Order error: {0}")]
    Order(String),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
