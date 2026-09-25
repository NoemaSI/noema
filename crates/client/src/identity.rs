use std::fs;
use std::path::Path;

use ed25519_dalek::{Signer, SigningKey, VerifyingKey, PUBLIC_KEY_LENGTH, SECRET_KEY_LENGTH};
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// A client identity backed by an ed25519 keypair.
///
/// The signing key is only ever written to disk via [`Identity::load_or_create`].
pub struct Identity {
    signing_key: SigningKey,
}

impl Identity {
    /// Generate a fresh identity from the OS random source.
    pub fn generate() -> Result<Self, Error> {
        let mut secret = [0u8; SECRET_KEY_LENGTH];
        getrandom::fill(&mut secret).map_err(|e| Error::Rng(e.to_string()))?;
        Ok(Self {
            signing_key: SigningKey::from_bytes(&secret),
        })
    }

    /// Load the signing key from `path`, or create it if the file does not exist.
    pub fn load_or_create(path: &Path) -> Result<Self, Error> {
        if path.exists() {
            let bytes = fs::read(path)?;
            let secret: [u8; SECRET_KEY_LENGTH] =
                bytes
                    .try_into()
                    .map_err(|b: Vec<u8>| Error::InvalidKeyFile {
                        expected: SECRET_KEY_LENGTH,
                        actual: b.len(),
                    })?;
            Ok(Self {
                signing_key: SigningKey::from_bytes(&secret),
            })
        } else {
            let identity = Self::generate()?;
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)?;
                }
            }
            fs::write(path, identity.signing_key.to_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            }
            Ok(identity)
        }
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    pub fn secret_bytes(&self) -> [u8; SECRET_KEY_LENGTH] {
        self.signing_key.to_bytes()
    }

    pub fn from_secret_bytes(secret: &[u8; SECRET_KEY_LENGTH]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(secret),
        }
    }

    pub fn public_key_bytes(&self) -> [u8; PUBLIC_KEY_LENGTH] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Hex-encoded public key — the shareable identifier for this identity.
    pub fn public_key_hex(&self) -> String {
        hex_encode(&self.public_key_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> ed25519_dalek::Signature {
        self.signing_key.sign(message)
    }
}
/// On-disk JSON format for a named identity.
/// Keys are stored as hex strings (older files stored raw byte arrays).
#[derive(Serialize, Deserialize)]
struct IdentityFile {
    name: String,
    #[serde(deserialize_with = "hex_or_bytes", serialize_with = "bytes_as_hex")]
    secret_key: Vec<u8>,
    #[serde(default, deserialize_with = "hex_or_bytes", serialize_with = "bytes_as_hex")]
    public_key: Vec<u8>,
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>, Error> {
    if s.len() % 2 != 0 {
        return Err(Error::InvalidHex(s.to_string()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| Error::InvalidHex(s.to_string()))
        })
        .collect()
}

fn hex_or_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum HexOrBytes {
        Hex(String),
        Bytes(Vec<u8>),
    }
    match HexOrBytes::deserialize(deserializer)? {
        HexOrBytes::Hex(s) => hex_decode(&s).map_err(serde::de::Error::custom),
        HexOrBytes::Bytes(b) => Ok(b),
    }
}

fn bytes_as_hex<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&hex_encode(bytes))
}

/// An ed25519 identity with a goofy-animals name, persisted as JSON.
pub struct StoredIdentity {
    pub name: String,
    pub identity: Identity,
}

impl StoredIdentity {
    /// Load a named identity from `path`.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let bytes = fs::read(path)?;
        let file: IdentityFile = serde_json::from_slice(&bytes)?;
        let secret: [u8; SECRET_KEY_LENGTH] =
            file.secret_key
                .try_into()
                .map_err(|b: Vec<u8>| Error::InvalidKeyFile {
                    expected: SECRET_KEY_LENGTH,
                    actual: b.len(),
                })?;
        let identity = Identity::from_secret_bytes(&secret);
        if !file.public_key.is_empty() && file.public_key != identity.public_key_bytes() {
            return Err(Error::PublicKeyMismatch);
        }
        Ok(Self {
            name: file.name,
            identity,
        })
    }

    pub fn public_key_hex(&self) -> String {
        self.identity.public_key_hex()
    }

    /// Create a fresh identity with a random goofy name and persist it to `path`.
    /// Loads the existing file instead if one is already there.
    pub fn create(path: &Path) -> Result<Self, Error> {
        if path.exists() {
            return Self::load(path);
        }
        let name = goofy_name()?;
        let identity = Identity::generate()?;
        let file = IdentityFile {
            name: name.clone(),
            secret_key: identity.secret_bytes().to_vec(),
            public_key: identity.public_key_bytes().to_vec(),
        };
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, serde_json::to_vec_pretty(&file)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(Self { name, identity })
    }
}

fn goofy_name() -> Result<String, Error> {
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::from_os_rng();
    Ok(goofy_animals::generate_name(&mut rng))
}
