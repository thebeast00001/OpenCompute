//! Identity for OpenCompute: device keys, where they are stored, the signed
//! roster of paired devices, and the pairing handshake.
//!
//! The design is in `docs/PLAN.md`, section 4.1.

pub mod keys;
pub mod keystore;

pub use keys::{DeviceKey, RootKey, SIGNATURE_LEN, verify_signature};
pub use keystore::{FileStore, KeychainStore, MemoryStore, SecretStore, StoreError};
