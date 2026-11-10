//! What a device is allowed to do in its cluster.

use std::fmt;
use std::str::FromStr;

/// A device's role, recorded in the signed roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Manages devices, submits work and accepts work. Holds the root key.
    Admin,
    /// Submits work and accepts work.
    Member,
    /// Accepts work only.
    Worker,
    /// Submits work only. For phones and thin laptops.
    Client,
}

impl Role {
    /// Every role, in order of decreasing privilege.
    pub const ALL: [Role; 4] = [Role::Admin, Role::Member, Role::Worker, Role::Client];

    /// May add and revoke devices.
    pub fn can_manage_devices(self) -> bool {
        matches!(self, Role::Admin)
    }

    /// May submit work to other devices.
    pub fn can_submit(self) -> bool {
        matches!(self, Role::Admin | Role::Member | Role::Client)
    }

    /// May run work for other devices.
    pub fn can_accept_work(self) -> bool {
        matches!(self, Role::Admin | Role::Member | Role::Worker)
    }

    /// The lower-case name used in the CLI and in configuration.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
            Role::Worker => "worker",
            Role::Client => "client",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The text is not the name of a role.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown role {0:?}; expected admin, member, worker or client")]
pub struct UnknownRole(pub String);

impl FromStr for Role {
    type Err = UnknownRole;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Role::ALL
            .into_iter()
            .find(|role| role.as_str().eq_ignore_ascii_case(text.trim()))
            .ok_or_else(|| UnknownRole(text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for role in Role::ALL {
            assert_eq!(role.to_string().parse::<Role>().unwrap(), role);
        }
        assert_eq!(" Admin ".parse::<Role>().unwrap(), Role::Admin);
        assert!("owner".parse::<Role>().is_err());
    }

    #[test]
    fn permissions_match_the_plan() {
        assert!(Role::Admin.can_manage_devices());
        assert!(!Role::Member.can_manage_devices());
        assert!(Role::Worker.can_accept_work() && !Role::Worker.can_submit());
        assert!(Role::Client.can_submit() && !Role::Client.can_accept_work());
    }
}
