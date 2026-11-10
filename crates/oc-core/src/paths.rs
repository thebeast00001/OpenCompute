//! Where an agent keeps its files.
//!
//! Each agent instance owns one data directory. Several agents can run on one
//! machine as long as each is given its own directory, which is how the
//! project is tested on a single computer.

use std::path::PathBuf;

/// Environment variable that overrides the data directory.
pub const DATA_DIR_ENV: &str = "OCX_DATA_DIR";

/// File names inside a data directory.
pub mod file {
    /// Optional configuration.
    pub const CONFIG: &str = "config.toml";
    /// SQLite database.
    pub const DATABASE: &str = "agent.db";
    /// Port and token of the local control channel, written while the agent runs.
    pub const CONTROL: &str = "control.json";
    /// Random identifier of this instance, used to name its keychain entries.
    pub const INSTANCE: &str = "instance";
    /// Directory for key files when the file key store is in use.
    pub const KEYS_DIR: &str = "keys";
    /// Directory for log files.
    pub const LOGS_DIR: &str = "logs";
    /// Log written by an agent started in the background.
    pub const AGENT_LOG: &str = "ocxd.log";
}

/// The operating-system family, as far as file locations are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Windows.
    Windows,
    /// macOS.
    MacOs,
    /// Linux and other Unix systems.
    Unix,
}

impl Platform {
    /// The platform this binary was built for.
    pub const fn current() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Unix
        }
    }
}

/// No data directory could be chosen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "cannot work out where to keep data: {missing} is not set. \
     Pass --data-dir or set {DATA_DIR_ENV}."
)]
pub struct NoDataDir {
    /// The environment variable that was needed.
    pub missing: &'static str,
}

/// The default data directory for a platform, given a way to read
/// environment variables.
pub fn default_data_dir(
    platform: Platform,
    env: impl Fn(&str) -> Option<String>,
) -> Result<PathBuf, NoDataDir> {
    let var = |name: &'static str| {
        env(name)
            .filter(|v| !v.is_empty())
            .ok_or(NoDataDir { missing: name })
    };
    match platform {
        Platform::Windows => Ok(PathBuf::from(var("LOCALAPPDATA")?).join("OpenCompute")),
        Platform::MacOs => Ok(PathBuf::from(var("HOME")?)
            .join("Library")
            .join("Application Support")
            .join("OpenCompute")),
        Platform::Unix => match env("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
            Some(xdg) => Ok(PathBuf::from(xdg).join("opencompute")),
            None => Ok(PathBuf::from(var("HOME")?)
                .join(".local")
                .join("share")
                .join("opencompute")),
        },
    }
}

/// Chooses the data directory: an explicit flag wins, then the
/// `OCX_DATA_DIR` environment variable, then the platform default.
pub fn resolve_data_dir(
    flag: Option<PathBuf>,
    platform: Platform,
    env: impl Fn(&str) -> Option<String>,
) -> Result<PathBuf, NoDataDir> {
    if let Some(dir) = flag {
        return Ok(dir);
    }
    if let Some(dir) = env(DATA_DIR_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    default_data_dir(platform, env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn flag_beats_environment_beats_default() {
        let env = env_of(&[(DATA_DIR_ENV, "from-env"), ("HOME", "home")]);
        let dir = resolve_data_dir(Some("from-flag".into()), Platform::Unix, &env).unwrap();
        assert_eq!(dir, PathBuf::from("from-flag"));
        let dir = resolve_data_dir(None, Platform::Unix, &env).unwrap();
        assert_eq!(dir, PathBuf::from("from-env"));
        let env = env_of(&[("HOME", "home")]);
        let dir = resolve_data_dir(None, Platform::Unix, &env).unwrap();
        assert_eq!(
            dir,
            PathBuf::from("home")
                .join(".local")
                .join("share")
                .join("opencompute")
        );
    }

    #[test]
    fn platform_defaults() {
        let env = env_of(&[
            ("LOCALAPPDATA", "appdata"),
            ("HOME", "home"),
            ("XDG_DATA_HOME", "xdg"),
        ]);
        assert_eq!(
            default_data_dir(Platform::Windows, &env).unwrap(),
            PathBuf::from("appdata").join("OpenCompute")
        );
        assert_eq!(
            default_data_dir(Platform::MacOs, &env).unwrap(),
            PathBuf::from("home")
                .join("Library")
                .join("Application Support")
                .join("OpenCompute")
        );
        assert_eq!(
            default_data_dir(Platform::Unix, &env).unwrap(),
            PathBuf::from("xdg").join("opencompute")
        );
    }

    #[test]
    fn a_missing_variable_is_named() {
        let env = env_of(&[]);
        let err = default_data_dir(Platform::Windows, &env).unwrap_err();
        assert_eq!(err.missing, "LOCALAPPDATA");
        assert!(err.to_string().contains("--data-dir"));
        // An empty value counts as missing.
        let env = env_of(&[("HOME", "")]);
        assert!(default_data_dir(Platform::Unix, &env).is_err());
    }
}
