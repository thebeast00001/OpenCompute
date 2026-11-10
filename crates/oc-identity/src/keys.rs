//! Ed25519 keys: one per device, and one root key per cluster.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use oc_core::{ClusterId, DeviceId};
use zeroize::Zeroizing;

/// Length of an Ed25519 signature in bytes.
pub const SIGNATURE_LEN: usize = 64;

/// The operating system could not supply random bytes.
#[derive(Debug, thiserror::Error)]
#[error("the operating system could not supply random bytes: {0}")]
pub struct RandomError(String);

/// Fills a buffer from the operating system's random source.
pub fn random_bytes<const N: usize>() -> Result<[u8; N], RandomError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| RandomError(e.to_string()))?;
    Ok(bytes)
}

fn generate_signing_key() -> Result<SigningKey, RandomError> {
    let seed = Zeroizing::new(random_bytes::<32>()?);
    Ok(SigningKey::from_bytes(&seed))
}

/// Checks an Ed25519 signature made by the key with the given public bytes.
///
/// Uses strict verification, which rejects malleable signatures and weak
/// public keys, so two devices can never disagree about whether a signed
/// record is valid.
pub fn verify_signature(public_key: &[u8; 32], message: &[u8], signature: &[u8]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(signature) else {
        return false;
    };
    key.verify_strict(message, &signature).is_ok()
}

/// A device's secret key. Its public half is the device's permanent
/// identity and network address.
pub struct DeviceKey(SigningKey);

impl DeviceKey {
    /// Creates a new random key.
    pub fn generate() -> Result<Self, RandomError> {
        generate_signing_key().map(Self)
    }

    /// Rebuilds a key from its 32 secret bytes.
    pub fn from_secret(secret: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(secret))
    }

    /// The 32 secret bytes, for storage and for the transport's TLS identity.
    pub fn secret_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    /// The device's public identity.
    pub fn id(&self) -> DeviceId {
        DeviceId::from_bytes(self.0.verifying_key().to_bytes())
    }

    /// Signs a message.
    pub fn sign(&self, message: &[u8]) -> [u8; SIGNATURE_LEN] {
        self.0.sign(message).to_bytes()
    }
}

impl std::fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DeviceKey({})", self.id().short())
    }
}

/// A cluster's root key. It signs every roster entry; devices with the admin
/// role hold a copy.
pub struct RootKey(SigningKey);

impl RootKey {
    /// Creates a new random key.
    pub fn generate() -> Result<Self, RandomError> {
        generate_signing_key().map(Self)
    }

    /// Rebuilds a key from its 32 secret bytes.
    pub fn from_secret(secret: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(secret))
    }

    /// The 32 secret bytes, for storage and for handing to a new admin.
    pub fn secret_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    /// The cluster's public identity.
    pub fn cluster_id(&self) -> ClusterId {
        ClusterId::from_bytes(self.0.verifying_key().to_bytes())
    }

    /// Signs a message.
    pub fn sign(&self, message: &[u8]) -> [u8; SIGNATURE_LEN] {
        self.0.sign(message).to_bytes()
    }
}

impl std::fmt::Debug for RootKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RootKey({})", self.cluster_id().short())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_survives_storage() {
        let key = DeviceKey::generate().unwrap();
        let restored = DeviceKey::from_secret(&key.secret_bytes());
        assert_eq!(key.id(), restored.id());
    }

    #[test]
    fn generated_keys_differ() {
        let a = DeviceKey::generate().unwrap();
        let b = DeviceKey::generate().unwrap();
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn signatures_verify_only_for_the_signer_and_message() {
        let key = RootKey::generate().unwrap();
        let other = RootKey::generate().unwrap();
        let signature = key.sign(b"add device");
        let public = *key.cluster_id().as_bytes();
        assert!(verify_signature(&public, b"add device", &signature));
        assert!(!verify_signature(&public, b"add devicf", &signature));
        assert!(!verify_signature(
            other.cluster_id().as_bytes(),
            b"add device",
            &signature
        ));
    }

    #[test]
    fn malformed_input_does_not_verify() {
        let key = DeviceKey::generate().unwrap();
        let signature = key.sign(b"m");
        let public = *key.id().as_bytes();
        assert!(!verify_signature(&public, b"m", &signature[..63]));
        assert!(!verify_signature(&public, b"m", &[0u8; 64]));
        // Not every 32-byte string is a valid public key.
        assert!(!verify_signature(&[0xFF; 32], b"m", &signature));
    }

    #[test]
    fn debug_never_prints_the_secret() {
        let key = DeviceKey::from_secret(&[7u8; 32]);
        let debug = format!("{key:?}");
        assert!(debug.starts_with("DeviceKey("));
        assert!(!debug.contains("0707"));
    }
}
