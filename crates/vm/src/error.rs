#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("smolmachines error: {0}")]
    SmolMachines(#[from] smolmachines::Error),

    #[error("runtime initialization failed: {0}")]
    Runtime(String),
}

pub type Result<T> = std::result::Result<T, Error>;