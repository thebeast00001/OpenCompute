//! Identifiers for devices and clusters.
//!
//! Both are Ed25519 public keys. A device's key is also its network address
//! (iroh's `EndpointId`); a cluster's key is the root key that signs its
//! roster. They are distinct types so one cannot be passed where the other is
//! expected.

use std::fmt;
use std::str::FromStr;

/// Why a textual identifier could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdParseError {
    /// The text is not 64 characters long.
    #[error("expected 64 hexadecimal characters, found {0}")]
    Length(usize),
    /// The text contains something other than `0-9`, `a-f` or `A-F`.
    #[error("not a hexadecimal character: {0:?}")]
    Character(char),
}

fn decode_hex(text: &str) -> Result<[u8; 32], IdParseError> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return Err(IdParseError::Length(text.chars().count()));
    }
    let nibble = |b: u8| match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        other => Err(IdParseError::Character(other as char)),
    };
    let mut out = [0u8; 32];
    let (pairs, _) = bytes.as_chunks::<2>();
    for (slot, [high, low]) in out.iter_mut().zip(pairs) {
        *slot = (nibble(*high)? << 4) | nibble(*low)?;
    }
    Ok(out)
}

fn write_hex(bytes: &[u8], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for byte in bytes {
        write!(f, "{byte:02x}")?;
    }
    Ok(())
}

macro_rules! key_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            /// Length of the identifier in bytes.
            pub const LEN: usize = 32;

            /// Wraps raw public-key bytes.
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// Wraps a byte slice, which must be exactly 32 bytes long.
            pub fn from_slice(bytes: &[u8]) -> Option<Self> {
                <[u8; 32]>::try_from(bytes).ok().map(Self)
            }

            /// The raw public-key bytes.
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            /// The first ten hexadecimal characters, for logs and tables.
            pub fn short(&self) -> String {
                self.to_string()[..10].to_owned()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_hex(&self.0, f)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}(", stringify!($name))?;
                write_hex(&self.0[..5], f)?;
                write!(f, "…)")
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                decode_hex(text.trim()).map(Self)
            }
        }
    };
}

key_id! {
    /// A device's permanent identity: its Ed25519 public key.
    DeviceId
}

key_id! {
    /// A cluster's identity: the public half of its root key.
    ClusterId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_parse_round_trip() {
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i * 7 + 3) as u8;
        }
        let id = DeviceId::from_bytes(bytes);
        let text = id.to_string();
        assert_eq!(text.len(), 64);
        assert_eq!(text.parse::<DeviceId>().unwrap(), id);
        assert_eq!(text.to_uppercase().parse::<DeviceId>().unwrap(), id);
        assert_eq!(id.short(), &text[..10]);
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert_eq!("abc".parse::<DeviceId>(), Err(IdParseError::Length(3)));
        let bad = "g".repeat(64);
        assert_eq!(bad.parse::<DeviceId>(), Err(IdParseError::Character('g')));
        // A multi-byte character must not be sliced in half.
        let wide = "é".repeat(32);
        assert!(wide.parse::<DeviceId>().is_err());
    }

    #[test]
    fn from_slice_checks_length() {
        assert!(ClusterId::from_slice(&[1u8; 32]).is_some());
        assert!(ClusterId::from_slice(&[1u8; 31]).is_none());
    }

    #[test]
    fn debug_does_not_print_the_whole_key() {
        let id = DeviceId::from_bytes([0xAB; 32]);
        assert_eq!(format!("{id:?}"), "DeviceId(ababababab…)");
    }
}
