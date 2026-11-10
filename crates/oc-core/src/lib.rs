//! Shared types for OpenCompute: identifiers, errors, configuration and
//! resource descriptions.
//!
//! Nothing in this crate performs I/O. The design is in `docs/PLAN.md`,
//! section 4.

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
