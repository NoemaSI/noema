use crate::error::Error;
use crate::protocol::{ClientMessage, RegistrationRequest};

/// Pluggable network layer. The concrete implementation (HTTP, TCP, ...)
/// comes once the server exists.
pub trait Transport {
    fn register(&mut self, request: &RegistrationRequest) -> Result<(), Error>;

    fn send(&mut self, message: &ClientMessage) -> Result<(), Error>;
}