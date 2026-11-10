//! Where secret keys are kept.
//!
//! The default is the operating system's keychain (Windows Credential
//! Manager, macOS Keychain, the Secret Service on Linux). A file store exists
//! for extra test instances on one machine and for machines with no keychain.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use oc_core::config::KeyStoreMode;
use zeroize::Zeroizing;

use crate::keys::{DeviceKey, RandomError, RootKey};

/// Name under which the device key is stored.
pub const DEVICE_KEY: &str = "device";
/// Name under which the cluster root key is stored, on admin devices.
pub const ROOT_KEY: &str = "root";

const KEYCHAIN_SERVICE: &str = "OpenCompute";

/// A 32-byte secret that is wiped from memory when dropped.
pub type Secret = Zeroizing<[u8; 32]>;

/// A key store that could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The OS keychain refused or is not running.
    #[error("the OS keychain is not available: {0}")]
    KeychainUnavailable(String),
    /// A key file could not be read or written.
    #[error("key file {path}: {source}")]
    File {
        /// The file involved.
        path: PathBuf,
        /// What went wrong.
        source: std::io::Error,
    },
    /// The stored value is not a 32-byte key.
    #[error("the stored {0} key is damaged")]
    Damaged(String),
    /// No random bytes were available to create a key.
    #[error(transparent)]
    Random(#[from] RandomError),
}

/// A place that keeps named 32-byte secrets.
pub trait SecretStore: Send + Sync {
    /// Reads a secret, or `None` if it was never stored.
    fn load(&self, name: &str) -> Result<Option<Secret>, StoreError>;
    /// Stores a secret, replacing any earlier value.
    fn save(&self, name: &str, secret: &[u8; 32]) -> Result<(), StoreError>;
    /// Removes a secret. Removing one that does not exist is not an error.
    fn delete(&self, name: &str) -> Result<(), StoreError>;
    /// A short description for logs and `ocx status`.
    fn describe(&self) -> &'static str;
}

// ------------------------------------------------------------------ memory

/// Keeps secrets in memory only. For tests.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, [u8; 32]>>);

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemoryStore {
    fn load(&self, name: &str) -> Result<Option<Secret>, StoreError> {
        let map = self.0.lock().expect("memory store lock");
        Ok(map.get(name).map(|s| Zeroizing::new(*s)))
    }

    fn save(&self, name: &str, secret: &[u8; 32]) -> Result<(), StoreError> {
        let mut map = self.0.lock().expect("memory store lock");
        map.insert(name.to_owned(), *secret);
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), StoreError> {
        self.0.lock().expect("memory store lock").remove(name);
        Ok(())
    }

    fn describe(&self) -> &'static str {
        "memory"
    }
}

// -------------------------------------------------------------------- file

/// Keeps each secret as a small hexadecimal file in one directory.
///
/// On Unix the files are created readable by their owner only. On Windows
/// they inherit the permissions of the data directory, which lives in the
/// user's profile.
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    /// A store that keeps its files in `dir`. The directory is created on
    /// first save.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.key"))
    }

    /// Whether this directory already holds the named secret.
    pub fn contains(&self, name: &str) -> bool {
        self.path(name).is_file()
    }
}

fn file_error(path: &Path) -> impl FnOnce(std::io::Error) -> StoreError + '_ {
    move |source| StoreError::File {
        path: path.to_owned(),
        source,
    }
}

fn to_hex(bytes: &[u8; 32]) -> Zeroizing<String> {
    let mut text = Zeroizing::new(String::with_capacity(64));
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn from_hex(text: &str) -> Option<Secret> {
    let text = text.trim();
    if text.len() != 64 {
        return None;
    }
    let nibble = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Zeroizing::new([0u8; 32]);
    let (pairs, _) = text.as_bytes().as_chunks::<2>();
    for (slot, [high, low]) in out.iter_mut().zip(pairs) {
        *slot = (nibble(*high)? << 4) | nibble(*low)?;
    }
    Some(out)
}

impl SecretStore for FileStore {
    fn load(&self, name: &str) -> Result<Option<Secret>, StoreError> {
        let path = self.path(name);
        let text = match fs::read_to_string(&path) {
            Ok(text) => Zeroizing::new(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(file_error(&path)(e)),
        };
        from_hex(&text)
            .map(Some)
            .ok_or_else(|| StoreError::Damaged(name.to_owned()))
    }

    fn save(&self, name: &str, secret: &[u8; 32]) -> Result<(), StoreError> {
        fs::create_dir_all(&self.dir).map_err(file_error(&self.dir))?;
        let path = self.path(name);
        // Write a temporary file and rename it, so a crash never leaves half
        // a key behind.
        let tmp = self.dir.join(format!("{name}.key.tmp"));
        {
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp).map_err(file_error(&tmp))?;
            let hex = to_hex(secret);
            file.write_all(hex.as_bytes()).map_err(file_error(&tmp))?;
            file.write_all(b"\n").map_err(file_error(&tmp))?;
            file.sync_all().map_err(file_error(&tmp))?;
        }
        fs::rename(&tmp, &path).map_err(file_error(&path))
    }

    fn delete(&self, name: &str) -> Result<(), StoreError> {
        let path = self.path(name);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(file_error(&path)(e)),
        }
    }

    fn describe(&self) -> &'static str {
        "file"
    }
}

// ---------------------------------------------------------------- keychain

/// Keeps secrets in the operating system's keychain.
///
/// Entries are named after the agent instance, so several instances on one
/// machine never read each other's keys.
pub struct KeychainStore {
    instance: String,
}

impl KeychainStore {
    /// Opens the keychain for the given instance and checks that it answers.
    pub fn open(instance: &str) -> Result<Self, StoreError> {
        let store = Self {
            instance: instance.to_owned(),
        };
        // A read of an entry that may not exist proves the keychain works
        // without changing anything in it.
        store.load(DEVICE_KEY)?;
        Ok(store)
    }

    fn entry(&self, name: &str) -> Result<keyring::Entry, StoreError> {
        let account = format!("{name}-key:{}", self.instance);
        keyring::Entry::new(KEYCHAIN_SERVICE, &account).map_err(keychain_error)
    }
}

fn keychain_error(error: keyring::Error) -> StoreError {
    StoreError::KeychainUnavailable(error.to_string())
}

impl SecretStore for KeychainStore {
    fn load(&self, name: &str) -> Result<Option<Secret>, StoreError> {
        match self.entry(name)?.get_secret() {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                let secret = <[u8; 32]>::try_from(bytes.as_slice())
                    .map_err(|_| StoreError::Damaged(name.to_owned()))?;
                Ok(Some(Zeroizing::new(secret)))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain_error(e)),
        }
    }

    fn save(&self, name: &str, secret: &[u8; 32]) -> Result<(), StoreError> {
        self.entry(name)?.set_secret(secret).map_err(keychain_error)
    }

    fn delete(&self, name: &str) -> Result<(), StoreError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain_error(e)),
        }
    }

    fn describe(&self) -> &'static str {
        "OS keychain"
    }
}

// ----------------------------------------------------------------- opening

/// A store chosen by [`open_store`], and anything the user should be told
/// about the choice.
pub struct OpenedStore {
    /// The store to use.
    pub store: Box<dyn SecretStore>,
    /// Set when `auto` had to fall back to a file because the keychain did
    /// not answer.
    pub fallback_reason: Option<String>,
}

/// Opens the key store an agent instance should use.
///
/// `keys_dir` is where the file store keeps its files, and `instance`
/// distinguishes this instance's keychain entries from another's.
///
/// In `auto` mode an instance that already has a key file keeps using it;
/// otherwise the keychain is used, and a file only if the keychain does not
/// answer.
pub fn open_store(
    mode: KeyStoreMode,
    keys_dir: &Path,
    instance: &str,
) -> Result<OpenedStore, StoreError> {
    let file = || Box::new(FileStore::new(keys_dir)) as Box<dyn SecretStore>;
    match mode {
        KeyStoreMode::File => Ok(OpenedStore {
            store: file(),
            fallback_reason: None,
        }),
        KeyStoreMode::Keychain => Ok(OpenedStore {
            store: Box::new(KeychainStore::open(instance)?),
            fallback_reason: None,
        }),
        KeyStoreMode::Auto => {
            if FileStore::new(keys_dir).contains(DEVICE_KEY) {
                return Ok(OpenedStore {
                    store: file(),
                    fallback_reason: None,
                });
            }
            match KeychainStore::open(instance) {
                Ok(store) => Ok(OpenedStore {
                    store: Box::new(store),
                    fallback_reason: None,
                }),
                Err(e) => Ok(OpenedStore {
                    store: file(),
                    fallback_reason: Some(e.to_string()),
                }),
            }
        }
    }
}

/// Loads the device key, creating and storing a new one on first run.
/// Returns the key and whether it was just created.
pub fn load_or_create_device_key(store: &dyn SecretStore) -> Result<(DeviceKey, bool), StoreError> {
    if let Some(secret) = store.load(DEVICE_KEY)? {
        return Ok((DeviceKey::from_secret(&secret), false));
    }
    let key = DeviceKey::generate()?;
    store.save(DEVICE_KEY, &key.secret_bytes())?;
    Ok((key, true))
}

/// Loads the cluster root key if this device holds one.
pub fn load_root_key(store: &dyn SecretStore) -> Result<Option<RootKey>, StoreError> {
    Ok(store
        .load(ROOT_KEY)?
        .map(|secret| RootKey::from_secret(&secret)))
}

/// Stores the cluster root key.
pub fn save_root_key(store: &dyn SecretStore, key: &RootKey) -> Result<(), StoreError> {
    store.save(ROOT_KEY, &key.secret_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(store: &dyn SecretStore) {
        assert!(store.load("device").unwrap().is_none());
        store.save("device", &[1u8; 32]).unwrap();
        store.save("root", &[2u8; 32]).unwrap();
        assert_eq!(*store.load("device").unwrap().unwrap(), [1u8; 32]);
        assert_eq!(*store.load("root").unwrap().unwrap(), [2u8; 32]);
        store.save("device", &[3u8; 32]).unwrap();
        assert_eq!(*store.load("device").unwrap().unwrap(), [3u8; 32]);
        store.delete("device").unwrap();
        store.delete("device").unwrap();
        assert!(store.load("device").unwrap().is_none());
        assert!(store.load("root").unwrap().is_some());
    }

    #[test]
    fn memory_store_behaves() {
        exercise(&MemoryStore::new());
    }

    #[test]
    fn file_store_behaves() {
        let dir = tempfile::tempdir().unwrap();
        // The directory does not exist yet; the first save creates it.
        exercise(&FileStore::new(dir.path().join("keys")));
    }

    #[test]
    fn file_store_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        FileStore::new(dir.path())
            .save("device", &[9u8; 32])
            .unwrap();
        let reopened = FileStore::new(dir.path());
        assert!(reopened.contains("device"));
        assert_eq!(*reopened.load("device").unwrap().unwrap(), [9u8; 32]);
        assert!(!dir.path().join("device.key.tmp").exists());
    }

    #[test]
    fn a_damaged_key_file_is_reported_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("device.key"), "not a key").unwrap();
        let store = FileStore::new(dir.path());
        assert!(matches!(
            store.load("device"),
            Err(StoreError::Damaged(name)) if name == "device"
        ));
        // Creating a new identity over a damaged one would silently change
        // the device's address, so loading must fail instead.
        assert!(load_or_create_device_key(&store).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn key_files_are_private_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        FileStore::new(dir.path())
            .save("device", &[9u8; 32])
            .unwrap();
        let mode = std::fs::metadata(dir.path().join("device.key"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn the_device_key_is_created_once() {
        let store = MemoryStore::new();
        let (first, created) = load_or_create_device_key(&store).unwrap();
        assert!(created);
        let (second, created) = load_or_create_device_key(&store).unwrap();
        assert!(!created);
        assert_eq!(first.id(), second.id());
    }

    #[test]
    fn the_root_key_round_trips() {
        let store = MemoryStore::new();
        assert!(load_root_key(&store).unwrap().is_none());
        let root = RootKey::generate().unwrap();
        save_root_key(&store, &root).unwrap();
        let loaded = load_root_key(&store).unwrap().unwrap();
        assert_eq!(loaded.cluster_id(), root.cluster_id());
    }

    #[test]
    fn file_mode_and_existing_key_files_never_touch_the_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let opened = open_store(KeyStoreMode::File, dir.path(), "test").unwrap();
        assert_eq!(opened.store.describe(), "file");
        opened.store.save(DEVICE_KEY, &[4u8; 32]).unwrap();
        // With a key file present, auto mode stays on the file store.
        let opened = open_store(KeyStoreMode::Auto, dir.path(), "test").unwrap();
        assert_eq!(opened.store.describe(), "file");
        assert!(opened.fallback_reason.is_none());
    }
}
