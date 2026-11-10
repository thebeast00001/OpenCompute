//! Agent configuration.
//!
//! The agent reads `config.toml` from its data directory. Every field has a
//! default, so the file is optional and may hold only the values the user
//! wants to change. Unknown keys are ignored, so a file written by a newer
//! release still loads on an older one.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Default UDP port for device-to-device traffic. A fixed port keeps firewall
/// rules predictable.
pub const DEFAULT_PORT: u16 = 7422;

/// Longest device name accepted, in bytes. It must fit in an mDNS TXT record
/// next to its key.
pub const MAX_DEVICE_NAME_LEN: usize = 63;

/// A configuration file that could not be used.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The text is not valid TOML or a value has the wrong type.
    #[error("config.toml is not valid: {0}")]
    Parse(#[from] toml::de::Error),
    /// The values are well-formed but do not make sense together.
    #[error("config.toml: {0}")]
    Invalid(String),
}

/// Everything the agent can be told through `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Name shown to other devices. Defaults to the host name.
    pub device_name: Option<String>,
    /// Device-to-device networking.
    pub network: NetworkConfig,
    /// Heartbeat timing.
    pub liveness: LivenessConfig,
    /// Where secret keys are kept.
    pub identity: IdentityConfig,
}

/// Device-to-device networking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// UDP port to listen on. `0` lets the OS choose.
    pub port: u16,
    /// Announce this device on the local network and look for others.
    pub mdns: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            mdns: true,
        }
    }
}

/// Heartbeat timing, in milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LivenessConfig {
    /// How often a heartbeat is sent to each peer.
    pub heartbeat_ms: u64,
    /// Silence after which a peer is shown as suspect.
    pub suspect_ms: u64,
    /// Silence after which a peer is treated as down.
    pub down_ms: u64,
}

impl Default for LivenessConfig {
    fn default() -> Self {
        Self {
            heartbeat_ms: 1_000,
            suspect_ms: 5_000,
            down_ms: 15_000,
        }
    }
}

impl LivenessConfig {
    /// Interval between heartbeats.
    pub fn heartbeat(&self) -> Duration {
        Duration::from_millis(self.heartbeat_ms)
    }

    /// Silence after which a peer is suspect.
    pub fn suspect_after(&self) -> Duration {
        Duration::from_millis(self.suspect_ms)
    }

    /// Silence after which a peer is down.
    pub fn down_after(&self) -> Duration {
        Duration::from_millis(self.down_ms)
    }
}

/// Where the device's secret keys are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyStoreMode {
    /// The OS keychain, falling back to a file in the data directory when the
    /// keychain is not available (for example on a server with no desktop
    /// session).
    #[default]
    Auto,
    /// The OS keychain only. Starting fails if it is not available.
    Keychain,
    /// A file in the data directory. Meant for extra test instances on one
    /// machine.
    File,
}

impl std::str::FromStr for KeyStoreMode {
    type Err = ConfigError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "keychain" => Ok(Self::Keychain),
            "file" => Ok(Self::File),
            other => Err(ConfigError::Invalid(format!(
                "unknown key store {other:?}; expected auto, keychain or file"
            ))),
        }
    }
}

/// Where secret keys are kept.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IdentityConfig {
    /// Which store holds the device key and, on admin devices, the cluster
    /// root key.
    pub key_store: KeyStoreMode,
}

impl AgentConfig {
    /// Parses and validates the contents of a `config.toml`.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    /// Checks that the values make sense together.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(name) = &self.device_name {
            validate_device_name(name).map_err(ConfigError::Invalid)?;
        }
        let l = &self.liveness;
        if l.heartbeat_ms == 0 {
            return Err(ConfigError::Invalid(
                "liveness.heartbeat_ms must be greater than zero".into(),
            ));
        }
        if !(l.heartbeat_ms < l.suspect_ms && l.suspect_ms < l.down_ms) {
            return Err(ConfigError::Invalid(
                "liveness values must satisfy heartbeat_ms < suspect_ms < down_ms".into(),
            ));
        }
        Ok(())
    }
}

/// Checks a device name: not empty, not too long, and free of control
/// characters (it is printed in terminals on other devices).
pub fn validate_device_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("device name must not be empty".into());
    }
    if name.len() > MAX_DEVICE_NAME_LEN {
        return Err(format!(
            "device name is longer than {MAX_DEVICE_NAME_LEN} bytes"
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("device name must not contain control characters".into());
    }
    Ok(())
}

/// Turns arbitrary text (a host name, a name received from a peer) into a
/// valid device name, or `None` if nothing usable is left.
pub fn sanitize_device_name(raw: &str) -> Option<String> {
    let mut name: String = raw.chars().filter(|c| !c.is_control()).collect();
    name = name.trim().to_owned();
    while name.len() > MAX_DEVICE_NAME_LEN {
        name.pop();
    }
    let name = name.trim_end().to_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_gives_the_defaults() {
        let config = AgentConfig::from_toml("").unwrap();
        assert_eq!(config, AgentConfig::default());
        assert_eq!(config.network.port, DEFAULT_PORT);
        assert!(config.network.mdns);
        assert_eq!(config.liveness.heartbeat(), Duration::from_secs(1));
        assert_eq!(config.liveness.suspect_after(), Duration::from_secs(5));
        assert_eq!(config.liveness.down_after(), Duration::from_secs(15));
        assert_eq!(config.identity.key_store, KeyStoreMode::Auto);
    }

    #[test]
    fn a_partial_file_overrides_only_what_it_names() {
        let config = AgentConfig::from_toml(
            r#"
            device_name = "workshop"
            [network]
            port = 0
            [identity]
            key_store = "file"
            "#,
        )
        .unwrap();
        assert_eq!(config.device_name.as_deref(), Some("workshop"));
        assert_eq!(config.network.port, 0);
        assert!(config.network.mdns);
        assert_eq!(config.identity.key_store, KeyStoreMode::File);
        assert_eq!(config.liveness, LivenessConfig::default());
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let config = AgentConfig::from_toml("from_a_newer_release = true\n").unwrap();
        assert_eq!(config, AgentConfig::default());
    }

    #[test]
    fn inconsistent_liveness_is_rejected() {
        let err = AgentConfig::from_toml("[liveness]\nsuspect_ms = 20000\n").unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)), "{err}");
        let err = AgentConfig::from_toml("[liveness]\nheartbeat_ms = 0\n").unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)), "{err}");
    }

    #[test]
    fn bad_types_are_reported() {
        let err = AgentConfig::from_toml("[network]\nport = \"high\"\n").unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)), "{err}");
    }

    #[test]
    fn device_names_are_checked_and_sanitized() {
        assert!(validate_device_name("den-pc").is_ok());
        assert!(validate_device_name("  ").is_err());
        assert!(validate_device_name("a\u{1b}[31m").is_err());
        assert!(validate_device_name(&"x".repeat(64)).is_err());
        assert_eq!(
            sanitize_device_name(" den\u{7}-pc ").as_deref(),
            Some("den-pc")
        );
        assert_eq!(sanitize_device_name("\n\t"), None);
        let long = sanitize_device_name(&"é".repeat(60)).unwrap();
        assert!(long.len() <= MAX_DEVICE_NAME_LEN);
        assert!(validate_device_name(&long).is_ok());
    }

    #[test]
    fn key_store_mode_parses_from_flags() {
        assert_eq!("FILE".parse::<KeyStoreMode>().unwrap(), KeyStoreMode::File);
        assert!("vault".parse::<KeyStoreMode>().is_err());
    }
}
