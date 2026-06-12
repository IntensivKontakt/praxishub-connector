use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConnectorError {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("TLS: {0}")]
    Tls(String),

    #[error("POP3: {0}")]
    Pop3(String),

    #[error("HTTP: {0}")]
    Http(String),

    #[error("Konfiguration: {0}")]
    Config(String),

    #[error("VDDS: {0}")]
    Vdds(String),

    #[error("EBZ: {0}")]
    Ebz(String),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, ConnectorError>;
