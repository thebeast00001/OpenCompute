//! Structured logging through `tracing`.

use tracing_subscriber::EnvFilter;

/// Environment variable holding a full filter, in `tracing`'s directive
/// syntax (for example `info,oc_net=debug`). It overrides `--log-level`.
pub const LOG_ENV: &str = "OCX_LOG";

/// Crates whose events follow the chosen level. Dependencies stay at `warn`
/// so their chatter does not bury the agent's own events.
const OUR_CRATES: [&str; 7] = [
    "oc_agent",
    "oc_core",
    "oc_identity",
    "oc_inventory",
    "oc_net",
    "oc_proto",
    "ocxd",
];

/// Builds the filter text for a level such as `info` or `debug`.
pub fn filter_for_level(level: &str) -> String {
    let mut filter = String::from("warn");
    for name in OUR_CRATES {
        filter.push_str(&format!(",{name}={level}"));
    }
    filter
}

/// Installs the global logger. Events go to standard error, as text or as
/// one JSON object per line.
pub fn init(level: &str, json: bool) -> anyhow::Result<()> {
    let directives = std::env::var(LOG_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| filter_for_level(level));
    let filter = EnvFilter::try_new(&directives)
        .map_err(|e| anyhow::anyhow!("invalid log filter {directives:?}: {e}"))?;
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    let installed = if json {
        builder.json().try_init()
    } else {
        builder.try_init()
    };
    installed.map_err(|e| anyhow::anyhow!("cannot install the logger: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_applies_to_our_crates_only() {
        let filter = filter_for_level("debug");
        assert!(filter.starts_with("warn,"));
        assert!(filter.contains("oc_agent=debug"));
        assert!(filter.contains("oc_net=debug"));
        assert!(EnvFilter::try_new(&filter).is_ok());
    }
}
