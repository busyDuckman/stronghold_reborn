// --------------------------------------------------------------------------------------------------------------------
// landscape_shader entry point. A shell around the library: start the log sink, run the app.

use anyhow::{Context, Result};

fn main() -> Result<()> {
    // RUST_LOG overrides the default; wgpu and naga are chatty at info, so they sit at warn.
    let _logger = flexi_logger::Logger::try_with_env_or_str("info, wgpu_core=warn, wgpu_hal=warn, naga=warn")
        .context("configuring the logger")?
        .start()
        .context("starting the logger")?;
    log::info!("{} {} starting", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    landscape_shader::app::run()
}
