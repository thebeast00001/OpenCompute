//! The OpenCompute agent as a library. The `ocxd` binary is a thin wrapper;
//! tests start several agents in one process through this crate.
//!
//! The design is in `docs/PLAN.md`, sections 3 and 4.

pub mod boot;
pub mod db;
pub mod logging;
