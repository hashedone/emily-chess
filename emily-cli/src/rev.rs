use std::path::PathBuf;

use color_eyre::eyre::OptionExt;
use shakmaty::Chess;
use structopt::StructOpt;
use tokio::fs::File;
use tokio::spawn;
use tracing::{debug, error, info, instrument};

use crate::adapters::TracingAdapt;
use crate::knowledge::Knowledge;
use crate::Config;
use color_eyre::Result;

use self::dispatcher::Dispatcher;

mod dispatcher;
mod engine;
mod processor;

/// Game review parameters
#[derive(Debug, StructOpt)]
pub struct Rev {
    /// Output PGN file
    #[structopt(short, long)]
    output: PathBuf,
}

impl Rev {
    #[instrument(skip(self, config), err)]
    pub async fn run(self, config: Config) -> Result<()> {
        debug!(?self, "Position review");

        let mut engine = engine::Engine::new(
            config.engine.ok_or_eyre("No engine configuration")?,
            &config.rev,
        )
        .await?;

        let root = Chess::new();
        info!(pos = %root.tr(), "Analyzing position");

        let mut knowledge = Knowledge::new(root.clone());

        let mut dispatcher = Dispatcher::builder();
        dispatcher.with(engine.new_game().await?);
        let dispatcher = dispatcher.build();
        dispatcher.dispatch(&mut knowledge, root).await?;

        spawn(async move {
            if let Err(err) = engine.quit().await {
                error!(?err, "Engine teardown failed");
            }
        });

        let mut output = File::create(&self.output).await?;
        knowledge.pgn()?.write_pgn(&mut output).await?;

        info!(file = ?self.output, "PGN stored");

        Ok(())
    }
}
