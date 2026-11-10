//! Everything an agent does before it touches the network: find its data
//! directory, read its configuration, open its database and load (or, on
//! first run, create) its identity.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use oc_core::AgentConfig;
use oc_core::config::{KeyStoreMode, validate_device_name};
use oc_core::paths::{self, Platform, file};
use oc_identity::DeviceKey;
use oc_identity::keys::random_bytes;
use oc_identity::keystore::{self, SecretStore};

use crate::db::Db;

/// Choices made on the command line. Each one overrides `config.toml`.
#[derive(Debug, Clone, Default)]
pub struct BootOptions {
    /// Data directory. Falls back to `OCX_DATA_DIR`, then the platform default.
    pub data_dir: Option<PathBuf>,
    /// Where secret keys are kept.
    pub key_store: Option<KeyStoreMode>,
    /// UDP port for device-to-device traffic.
    pub port: Option<u16>,
    /// Name shown to other devices.
    pub name: Option<String>,
    /// Turn local-network discovery off.
    pub no_mdns: bool,
}

/// A prepared agent instance: storage open, identity loaded, nothing running.
pub struct Boot {
    /// The instance's data directory.
    pub data_dir: PathBuf,
    /// Configuration after command-line overrides.
    pub config: AgentConfig,
    /// The open database.
    pub db: Db,
    /// Where secret keys are kept.
    pub secrets: Box<dyn SecretStore>,
    /// This device's key.
    pub key: DeviceKey,
    /// True when the key was created by this call (first run).
    pub key_created: bool,
    /// Set when the keychain did not answer and a key file is used instead.
    pub key_store_fallback: Option<String>,
}

/// Prepares an agent instance. Safe to call again on the same directory: the
/// second call finds the same identity and data.
///
/// Call this before starting an async runtime; some OS keychains block.
pub fn boot(options: BootOptions) -> Result<Boot> {
    let data_dir =
        paths::resolve_data_dir(options.data_dir.clone(), Platform::current(), |name| {
            std::env::var(name).ok()
        })?;
    fs::create_dir_all(&data_dir)
        .with_context(|| format!("cannot create data directory {}", data_dir.display()))?;

    let config = load_config(&data_dir, &options)?;
    let db = Db::open(&data_dir.join(file::DATABASE))
        .with_context(|| format!("cannot open the database in {}", data_dir.display()))?;

    let instance = instance_id(&data_dir)?;
    let opened = keystore::open_store(
        config.identity.key_store,
        &data_dir.join(file::KEYS_DIR),
        &instance,
    )?;
    let (key, key_created) = keystore::load_or_create_device_key(opened.store.as_ref())
        .context("cannot load this device's key")?;

    Ok(Boot {
        data_dir,
        config,
        db,
        secrets: opened.store,
        key,
        key_created,
        key_store_fallback: opened.fallback_reason,
    })
}

fn load_config(data_dir: &Path, options: &BootOptions) -> Result<AgentConfig> {
    let path = data_dir.join(file::CONFIG);
    let mut config = match fs::read_to_string(&path) {
        Ok(text) => AgentConfig::from_toml(&text)
            .with_context(|| format!("cannot use {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => AgentConfig::default(),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    if let Some(mode) = options.key_store {
        config.identity.key_store = mode;
    }
    if let Some(port) = options.port {
        config.network.port = port;
    }
    if let Some(name) = &options.name {
        if let Err(problem) = validate_device_name(name) {
            bail!("--name: {problem}");
        }
        config.device_name = Some(name.clone());
    }
    if options.no_mdns {
        config.network.mdns = false;
    }
    config.validate()?;
    Ok(config)
}

/// A random identifier for this data directory, created on first run. It
/// names the instance's keychain entries, so two instances on one machine
/// never share a key.
fn instance_id(data_dir: &Path) -> Result<String> {
    let path = data_dir.join(file::INSTANCE);
    match fs::read_to_string(&path) {
        Ok(text) => {
            let id = text.trim();
            if id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Ok(id.to_owned());
            }
            bail!(
                "{} is damaged. It names this instance's keychain entries, so it \
                 cannot be recreated automatically.",
                path.display()
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
    let id: String = random_bytes::<8>()?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    fs::write(&path, format!("{id}\n"))
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(dir: &Path) -> BootOptions {
        BootOptions {
            data_dir: Some(dir.to_owned()),
            // Tests must never read or write the developer's real keychain.
            key_store: Some(KeyStoreMode::File),
            ..BootOptions::default()
        }
    }

    #[test]
    fn first_run_creates_an_identity_and_later_runs_reuse_it() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("agent");

        let first = boot(options(&data)).unwrap();
        assert!(first.key_created);
        assert!(data.join(file::DATABASE).is_file());
        assert!(data.join(file::INSTANCE).is_file());
        assert!(data.join(file::KEYS_DIR).join("device.key").is_file());
        let id = first.key.id();
        drop(first);

        let second = boot(options(&data)).unwrap();
        assert!(!second.key_created);
        assert_eq!(second.key.id(), id);
    }

    #[test]
    fn two_data_directories_are_two_devices() {
        let dir = tempfile::tempdir().unwrap();
        let a = boot(options(&dir.path().join("a"))).unwrap();
        let b = boot(options(&dir.path().join("b"))).unwrap();
        assert_ne!(a.key.id(), b.key.id());
    }

    #[test]
    fn flags_override_the_config_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(file::CONFIG),
            "device_name = \"from-file\"\n[network]\nport = 9000\n",
        )
        .unwrap();

        let from_file = boot(options(dir.path())).unwrap();
        assert_eq!(from_file.config.device_name.as_deref(), Some("from-file"));
        assert_eq!(from_file.config.network.port, 9000);
        assert!(from_file.config.network.mdns);
        drop(from_file);

        let overridden = boot(BootOptions {
            port: Some(0),
            name: Some("from-flag".into()),
            no_mdns: true,
            ..options(dir.path())
        })
        .unwrap();
        assert_eq!(overridden.config.device_name.as_deref(), Some("from-flag"));
        assert_eq!(overridden.config.network.port, 0);
        assert!(!overridden.config.network.mdns);
    }

    #[test]
    fn a_bad_config_file_stops_the_agent_with_its_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(file::CONFIG), "[network]\nport = \"x\"\n").unwrap();
        let err = boot(options(dir.path())).err().unwrap();
        assert!(format!("{err:#}").contains(file::CONFIG), "{err:#}");
    }

    #[test]
    fn a_bad_name_flag_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let err = boot(BootOptions {
            name: Some("  ".into()),
            ..options(dir.path())
        })
        .err()
        .unwrap();
        assert!(format!("{err:#}").contains("--name"), "{err:#}");
    }

    #[test]
    fn the_instance_id_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let first = instance_id(dir.path()).unwrap();
        assert_eq!(first.len(), 16);
        assert_eq!(instance_id(dir.path()).unwrap(), first);
        fs::write(dir.path().join(file::INSTANCE), "oops").unwrap();
        assert!(instance_id(dir.path()).is_err());
    }
}
