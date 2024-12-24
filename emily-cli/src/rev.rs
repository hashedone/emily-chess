use structopt::StructOpt;
use tracing::{debug, instrument};

use crate::Config;
use color_eyre::Result;

/// Game review parameters
#[derive(Debug, StructOpt)]
pub struct Rev {}

impl Rev {
    #[instrument(skip(self, _config), err)]
    pub fn run(self, _config: Config) -> Result<()> {
        debug!(?self, "Position review");
        Ok(())
    }
}
