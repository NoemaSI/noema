pub mod error;
pub mod identity;
pub mod protocol;
pub mod transport;

pub use error::Error;
pub use identity::{Identity, StoredIdentity};
pub use protocol::{ClientMessage, RegistrationRequest};
pub use transport::Transport;
#[cfg(test)]
mod tests {
    use crate::{ClientMessage, Identity, RegistrationRequest, StoredIdentity};

    #[test]
    fn registration_roundtrip() {
        let identity = Identity::generate().unwrap();
        let request = RegistrationRequest::new(&identity);
        request.verify().unwrap();

        let bytes = request.to_bytes().unwrap();
        let decoded = RegistrationRequest::from_bytes(&bytes).unwrap();
        decoded.verify().unwrap();
    }

    #[test]
    fn message_roundtrip() {
        let identity = Identity::generate().unwrap();
        let message = ClientMessage::new(&identity, b"hello server".to_vec());
        message.verify().unwrap();

        let bytes = message.to_bytes().unwrap();
        let decoded = ClientMessage::from_bytes(&bytes).unwrap();
        decoded.verify().unwrap();
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let identity = Identity::generate().unwrap();
        let mut message = ClientMessage::new(&identity, b"hello server".to_vec());
        message.payload.push(0);
        assert!(message.verify().is_err());
    }

    #[test]
    fn load_or_create_is_stable() {
        let dir = std::env::temp_dir().join(format!("noema-client-test-{}", std::process::id()));
        let path = dir.join("noema-client.key");
        let first = Identity::load_or_create(&path).unwrap();
        let second = Identity::load_or_create(&path).unwrap();
        assert_eq!(first.public_key_bytes(), second.public_key_bytes());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stored_identity_roundtrip_hex() {
        let dir = std::env::temp_dir().join(format!("noema-stored-test-{}", std::process::id()));
        let path = dir.join("identity.json");
        let created = StoredIdentity::create(&path).unwrap();
        let json = std::fs::read_to_string(&path).unwrap();
        // keys must be hex strings, not byte arrays
        assert!(json.contains("\"public_key\""));
        assert!(!json.contains('['));

        let loaded = StoredIdentity::load(&path).unwrap();
        assert_eq!(created.name, loaded.name);
        assert_eq!(created.public_key_hex(), loaded.public_key_hex());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stored_identity_accepts_legacy_byte_arrays() {
        let dir = std::env::temp_dir().join(format!("noema-legacy-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("identity.json");
        let identity = Identity::generate().unwrap();
        let secret = identity.secret_bytes();
        let json = format!(
            r#"{{"name":"old-name","secret_key":[{}]}}"#,
            secret.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",")
        );
        std::fs::write(&path, json).unwrap();
        let loaded = StoredIdentity::load(&path).unwrap();
        assert_eq!(loaded.name, "old-name");
        assert_eq!(loaded.public_key_hex(), identity.public_key_hex());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
