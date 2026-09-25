use ed25519_dalek::{Signature, Verifier, VerifyingKey, PUBLIC_KEY_LENGTH};
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::identity::Identity;

/// Domain separation tags so signatures can't be replayed across message kinds.
pub const REGISTRATION_DOMAIN: &[u8] = b"noema:registration:v1\n";
pub const MESSAGE_DOMAIN: &[u8] = b"noema:message:v1\n";

/// Self-registration: the client proves possession of the private key
/// for the public key it is registering, without any server-issued challenge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationRequest {
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl RegistrationRequest {
    pub fn new(identity: &Identity) -> Self {
        let public_key = identity.public_key_bytes().to_vec();
        let signature = identity
            .sign(&registration_sig_bytes(&public_key))
            .to_bytes()
            .to_vec();
        Self { public_key, signature }
    }

    /// Server-side check (also usable by the client for testing).
    pub fn verify(&self) -> Result<(), Error> {
        let key = verifying_key_from_bytes(&self.public_key)?;
        let signature = Signature::try_from(self.signature.as_slice())?;
        key.verify(&registration_sig_bytes(&self.public_key), &signature)?;
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Ok(serde_json::to_vec(self)?)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

/// A signed payload sent to the server after registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMessage {
    pub public_key: Vec<u8>,
    pub payload: Vec<u8>,
    pub signature: Vec<u8>,
}

impl ClientMessage {
    pub fn new(identity: &Identity, payload: Vec<u8>) -> Self {
        let public_key = identity.public_key_bytes().to_vec();
        let signature = identity
            .sign(&message_sig_bytes(&payload))
            .to_bytes()
            .to_vec();
        Self {
            public_key,
            payload,
            signature,
        }
    }

    pub fn verify(&self) -> Result<(), Error> {
        let key = verifying_key_from_bytes(&self.public_key)?;
        let signature = Signature::try_from(self.signature.as_slice())?;
        key.verify(&message_sig_bytes(&self.payload), &signature)?;
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Ok(serde_json::to_vec(self)?)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

/// The exact bytes the registration signature covers.
pub fn registration_sig_bytes(public_key: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(REGISTRATION_DOMAIN.len() + public_key.len());
    bytes.extend_from_slice(REGISTRATION_DOMAIN);
    bytes.extend_from_slice(public_key);
    bytes
}

/// The exact bytes a message signature covers.
pub fn message_sig_bytes(payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(MESSAGE_DOMAIN.len() + payload.len());
    bytes.extend_from_slice(MESSAGE_DOMAIN);
    bytes.extend_from_slice(payload);
    bytes
}

fn verifying_key_from_bytes(bytes: &[u8]) -> Result<VerifyingKey, Error> {
    let bytes: [u8; PUBLIC_KEY_LENGTH] =
        bytes
            .to_vec()
            .try_into()
            .map_err(|b: Vec<u8>| Error::InvalidPublicKey {
                expected: PUBLIC_KEY_LENGTH,
                actual: b.len(),
            })?;
    Ok(VerifyingKey::from_bytes(&bytes)?)
}