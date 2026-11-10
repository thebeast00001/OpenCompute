//! Transport abstraction for OpenCompute.
//!
//! All networking sits behind one `Transport` trait with two implementations:
//! iroh for real devices, and an in-memory simulated network (configurable
//! delay, loss, bandwidth and partitions) for deterministic tests.
//!
//! Empty until Spike 1 (`spikes/net-iroh`) confirms the iroh API and its
//! behaviour on Windows. See `docs/PLAN.md`, section 4.2.
