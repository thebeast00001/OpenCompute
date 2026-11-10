//! `ocxd`, the OpenCompute agent. One runs on every device in a cluster.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use oc_agent::boot::{self, BootOptions};
use oc_agent::logging;
use oc_core::config::KeyStoreMode;

/// The OpenCompute agent.
#[derive(Debug, Parser)]
#[command(name = "ocxd", version = oc_core::VERSION, about)]
struct Args {
    /// Directory for this agent's database, keys and logs. Give each agent
    /// on one machine its own.
    #[arg(long, env = oc_core::paths::DATA_DIR_ENV, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Where to keep secret keys: auto, keychain or file.
    #[arg(long, value_name = "MODE")]
    key_store: Option<KeyStoreMode>,

    /// UDP port for device-to-device traffic. 0 lets the OS choose.
    #[arg(long)]
    port: Option<u16>,

    /// Name shown to other devices.
    #[arg(long)]
    name: Option<String>,

    /// Do not announce this device on the local network or look for others.
    #[arg(long)]
    no_mdns: bool,

    /// Log level for the agent's own events: error, warn, info, debug or trace.
    #[arg(long, default_value = "info", value_name = "LEVEL")]
    log_level: String,

    /// Write log events as JSON, one object per line.
    #[arg(long)]
    log_json: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    logging::init(&args.log_level, args.log_json)?;

    // Storage and identity come first, and before the async runtime starts:
    // some OS keychains block the calling thread.
    let boot = boot::boot(BootOptions {
        data_dir: args.data_dir,
        key_store: args.key_store,
        port: args.port,
        name: args.name,
        no_mdns: args.no_mdns,
    })?;

    if let Some(reason) = &boot.key_store_fallback {
        tracing::warn!(
            reason,
            "the OS keychain is not available; the device key is kept in a file \
             in the data directory instead"
        );
    }
    tracing::info!(
        device = %boot.key.id(),
        first_run = boot.key_created,
        key_store = boot.secrets.describe(),
        data_dir = %boot.data_dir.display(),
        "identity ready"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tracing::info!("agent running; press Ctrl-C to stop");
        tokio::signal::ctrl_c().await?;
        tracing::info!("stopping");
        Ok(())
    })
}
