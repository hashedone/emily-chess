use std::path::PathBuf;

use color_eyre::eyre::OptionExt;
use shakmaty::fen::Fen;
use shakmaty::{CastlingMode, Chess};
use structopt::StructOpt;
use tokio::fs::File;
use tokio::spawn;
use tracing::{error, info, instrument, trace};

use crate::adapters::debug::DFenExt;
use crate::knowledge::Knowledge;
use crate::Config;
use color_eyre::Result;

use self::dispatcher::Dispatcher;

mod dispatcher;
mod engine;
mod processor;

fn parse_chess(fen: &str) -> Result<Chess> {
    let fen: Fen = fen.parse()?;
    let fen: Chess = fen.into_position(CastlingMode::Standard)?;
    Ok(fen)
}

/// Game review parameters
#[derive(Debug, StructOpt)]
pub struct Rev {
    /// Output PGN file
    #[structopt(short, long)]
    output: PathBuf,
    /// Starting position
    #[structopt(short, long, parse(try_from_str = parse_chess))]
    fen: Option<Chess>,
}

impl Rev {
    #[instrument(skip(self, config), err)]
    pub async fn run(self, config: Config) -> Result<()> {
        info!(?self, "Position review");

        let mut engine = engine::Engine::new(
            config.engine.ok_or_eyre("No engine configuration")?,
            &config.rev,
        )
        .await?;

        let root = self.fen.unwrap_or_default();
        trace!(pos = ?root.d_fen(), "Analyzing position");

        let mut knowledge = Knowledge::new(root.clone());

        let mut dispatcher = Dispatcher::builder();
        dispatcher.with(engine.new_game().await?);
        let dispatcher = dispatcher.build();
        dispatcher.dispatch(&mut knowledge, 0, 0).await?;

        spawn(async move {
            if let Err(err) = engine.quit().await {
                error!(?err, "Engine teardown failed");
            }
        });

        let mut output = File::create(&self.output).await?;
        knowledge.pgn().write_pgn(&mut output).await?;

        info!(file = ?self.output, "PGN stored");

        Ok(())
    }
}
