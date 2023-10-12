use thiserror::Error;

#[derive(Debug, Error)]
pub enum BigTableError {
    #[error("Invalid Row Response: {0}")]
    InvalidRowResponse(String),

    #[error("Invalid Chunk: {0}")]
    InvalidChunk(String),

    #[error("BigTable read error {0}")]
    Read(String),

    #[error("BigTable write error {0}")]
    Write(String),

    #[error("BigTable Admin Error {0}")]
    Admin(String),

    #[error("Bigtable Recycle request")]
    Recycle,

    /// General Pool builder errors.
    #[error("Pool Error {0}")]
    Pool(String),
}
