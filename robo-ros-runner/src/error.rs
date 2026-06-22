#[derive(Debug, thiserror::Error)]
pub enum R3Error {
    #[error("Config error: {0}")]
    Config(String),
    #[error("CSV error: {0}")]
    Csv(String),
    #[error("Scene error: {0}")]
    Scene(String),
    #[error("URDF error: {0}")]
    Urdf(String),
    #[error("Mapping error: {0}")]
    Mapping(String),
    #[error("Topic error: {0}")]
    Topic(String),
    #[error("Transport error: {0}")]
    Transport(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
