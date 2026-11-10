//! Wire protocol definitions for OpenCompute.
//!
//! Each protocol between devices is its own QUIC application protocol (ALPN).
//! The trailing number is the major version; devices on the same major
//! version must interoperate. Message schemas (Protocol Buffers) are added in
//! Milestone 1. See `docs/PLAN.md`, section 4.3.

/// Major protocol version carried at the end of every ALPN.
pub const PROTOCOL_MAJOR: u32 = 1;

/// ALPN identifiers, one per protocol.
pub mod alpn {
    /// Version, capability and roster-version exchange on connect.
    pub const HELLO: &[u8] = b"oc/hello/1";
    /// Pairing handshake.
    pub const PAIR: &[u8] = b"oc/pair/1";
    /// Heartbeats, inventory, roster sync, leases and task control.
    pub const CTL: &[u8] = b"oc/ctl/1";
    /// Chunk requests and responses.
    pub const BLOB: &[u8] = b"oc/blob/1";
    /// Live stdout, stderr and log streaming.
    pub const IO: &[u8] = b"oc/io/1";
    /// TCP port forwarding for services.
    pub const TUNNEL: &[u8] = b"oc/tunnel/1";

    /// Every ALPN this build speaks.
    pub const ALL: [&[u8]; 6] = [HELLO, PAIR, CTL, BLOB, IO, TUNNEL];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_alpn_carries_the_major_version() {
        let suffix = format!("/{PROTOCOL_MAJOR}");
        for name in alpn::ALL {
            let name = std::str::from_utf8(name).expect("ALPNs are ASCII");
            assert!(name.starts_with("oc/"), "{name} lacks the oc/ prefix");
            assert!(name.ends_with(&suffix), "{name} lacks the version suffix");
        }
    }

    #[test]
    fn alpns_are_unique() {
        let mut names = alpn::ALL.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), alpn::ALL.len());
    }
}
