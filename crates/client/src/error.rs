#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("random number generation failed: {0}")]
    Rng(String),

    #[error("invalid key file: expected {expected} bytes, got {actual}")]
    InvalidKeyFile { expected: usize, actual: usize },

    #[error("invalid public key: expected {expected} bytes, got {actual}")]
    InvalidPublicKey { expected: usize, actual: usize },

    #[error("stored public key does not match secret key")]
    PublicKeyMismatch,

    #[error("invalid hex string: {0}")]
    InvalidHex(String),

    #[error("signature error: {0}")]
    Signature(#[from] ed25519_dalek::SignatureError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("transport error: {0}")]
    Transport(String),
}