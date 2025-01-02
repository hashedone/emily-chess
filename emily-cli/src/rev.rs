use std::path::PathBuf;
use std::time::Duration;

use color_eyre::eyre::OptionExt;
use shakmaty::{Chess, Color, Move, Position};
use structopt::StructOpt;
use tokio::fs::File;
use tokio::spawn;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, instrument};

use crate::adapters::TracingAdapt;
use crate::knowledge::Knowledge;
use crate::uci::{Engine, Score};
use crate::{config, Config};
use color_eyre::Result;

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

        let engine = EngineProcessor::new(
            config.engine.ok_or_eyre("No engine configuration")?,
            &config.rev,
        )
        .await?;

        let root = Chess::new();
        info!(pos = %root.tr(), "Analyzing position");

        let mut knowledge = Knowledge::new(root.clone());
        let (task, fen_tx, mut res_rx) = engine.spawn()?;

        fen_tx.send(root).await?;
        let mut total_moves = 1;
        let mut pending = 1;

        while let Some(res) = res_rx.recv().await {
            pending -= 1;

            let eval = match res.fen.turn() {
                Color::White => res.eval,
                Color::Black => res.eval.rev(),
            };

            info!(
                fen = res.fen.tr(),
                mov = %res.mov,
                %eval,
                pending,
                total = total_moves,
                "Move analysed"
            );

            knowledge.pos_mut(res.fen.clone()).update_eval(eval);
            let next_fen = knowledge
                .mov_mut(res.fen.clone(), res.mov)?
                .position()
                .clone();
            let next = knowledge.pos_mut(next_fen.clone());
            next.update_eval(eval);

            if next.outcome().is_none() && next.eval().is_none() {
                fen_tx.send(next_fen).await?;
                total_moves += 1;
                pending += 1;
            }

            if pending == 0 {
                break;
            }
        }

        drop(fen_tx);

        spawn(async move {
            let res = async move {
                let engine = task.await??;

                info!("Engine completed");
                engine.quit().await
            }
            .await;

            if let Err(err) = res {
                error!(?err, "Engine teardown failed");
            }
        });

        let mut output = File::create(&self.output).await?;
        knowledge.pgn()?.write_pgn(&mut output).await?;

        info!(file = ?self.output, "PGN stored");

        Ok(())
    }
}

/// Engine analysis details
#[derive(Debug)]
struct EngineAnalysis {
    /// Analysed position
    fen: Chess,
    /// Choosen move
    mov: Move,
    /// Engine evaluation
    eval: Score,
}

struct EngineProcessor {
    engine: Engine,
    name: String,
    depth: Option<u8>,
    time: Option<Duration>,
}

type EngineTask = JoinHandle<Result<EngineProcessor>>;

impl EngineProcessor {
    #[instrument(err)]
    async fn new(engine: config::Engine, config: &config::Rev) -> Result<Self> {
        let name = engine.name.clone();
        let engine = Engine::run(engine).await?;

        Ok(Self {
            engine,
            name,
            depth: config.depth,
            time: config.time,
        })
    }

    fn spawn(mut self) -> Result<(EngineTask, Sender<Chess>, Receiver<EngineAnalysis>)> {
        let (fen_tx, mut fen_rx) = mpsc::channel::<Chess>(10);
        let (res_tx, res_rx) = mpsc::channel::<EngineAnalysis>(2);

        let task = spawn(async move {
            self.engine.new_game().await?;

            while let Some(fen) = fen_rx.recv().await {
                let (mov, eval) = self.process(fen.clone()).await?;
                res_tx.send(EngineAnalysis { fen, mov, eval }).await?;
            }

            Ok(self)
        });

        Ok((task, fen_tx, res_rx))
    }

    #[instrument(skip_all, fields(engine=%self.name, fen=fen.tr()), err)]
    async fn process(&mut self, fen: Chess) -> Result<(Move, Score)> {
        let mut stream = self.engine.go(fen.clone(), self.depth, self.time).await?;

        let mut mov = None;
        let mut eval = None;

        while let Some(info) = stream.info().await? {
            mov = info.line.into_iter().next().or(mov);
            eval = Some(info.score);
        }

        let mov = mov.ok_or_eyre("No move after analysis")?;
        let mov = mov.to_move(&fen)?;
        let eval = eval.ok_or_eyre("No eval after analyis")?;

        debug!(%mov, %eval, "Position processed");
        Ok((mov, eval))
    }

    #[instrument(skip_all, fields(engine = %self.name), err)]
    async fn quit(self) -> Result<()> {
        self.engine.quit().await
    }
}
