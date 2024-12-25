use color_eyre::eyre::OptionExt;
use structopt::StructOpt;
use tracing::{debug, instrument};

use crate::uci::Engine;
use crate::Config;
use color_eyre::Result;

/// Game review parameters
#[derive(Debug, StructOpt)]
pub struct Rev {}

impl Rev {
    #[instrument(skip(self, config), err)]
    pub fn run(self, config: Config) -> Result<()> {
        debug!(?self, "Position review");

        let engine = config.engine.ok_or_eyre("No engine configuration")?;
        let _engine = Engine::run(engine)?;
        Ok(())
    }
}
