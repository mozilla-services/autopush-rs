use thiserror::Error;

#[derive(Debug, Error)]
pub enum BigTableError {
    #[error("Invalid Row Response: {0}")]
    InvalidRowResponse(String),

    #[error("Invalid Chunk: {0}")]
    InvalidChunk(String),

    #[error("BigTable read error {0}")]
    BigTableRead(String),

    #[error("BigTable write error {0}")]
    BigTableWrite(String),
}
