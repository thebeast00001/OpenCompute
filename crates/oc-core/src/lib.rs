//! Shared types for OpenCompute: identifiers, roles, configuration and time.
//!
//! Nothing in this crate touches the network, the disk or the OS keychain.
//! Functions that need the environment take it as an argument, so every one
//! of them can be tested without a real machine. The design is in
//! `docs/PLAN.md`, section 4.

pub mod config;
pub mod id;
pub mod paths;
pub mod role;
pub mod time;

pub use config::AgentConfig;
pub use id::{ClusterId, DeviceId, IdParseError};
pub use role::Role;

/// Version of this build, taken from the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!VERSION.is_empty());
    }
}
