// --------------------------------------------------------------------------------------------------------------------
// Stronghold entry point. A shell around the library; the logic lives in its modules.

use stronghold::prelude::*;

fn main() {
    // TODO: wire a log sink before shipping (flexi_logger is the house choice, see Cargo.toml).
    log::info!("{} starting", banner());
}
