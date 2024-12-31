use std::time::Duration;

use color_eyre::eyre::OptionExt;
use shakmaty::fen::Fen;
use shakmaty::uci::UciMove;
use shakmaty::{CastlingMode, Chess, Color, EnPassantMode, Position, Setup};
use structopt::StructOpt;
use tokio::spawn;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;
use tracing::{debug, info, instrument, Level};

use crate::knowledge::Knowledge;
use crate::uci::{Engine, Score};
use crate::{config, Config};
use color_eyre::Result;

/// Game review parameters
#[derive(Debug, StructOpt)]
pub struct Rev {}

impl Rev {
    #[instrument(skip(self, config), err)]
    pub async fn run(self, config: Config) -> Result<()> {
        debug!(?self, "Position review");

        let engine = EngineProcessor::new(
            config.engine.ok_or_eyre("No engine configuration")?,
            &config.rev,
        )
        .await?;

        let fen = Fen::from_setup(Setup::initial());
        info!("Analyzing position: {fen}");

        let _knowledge = Knowledge::new(fen.clone());
        let (task, fen_tx, mut res_rx) = engine.spawn()?;

        fen_tx.send(fen).await?;
        let mut total_moves = 1;
        let mut pending = 1;

        while let Some(res) = res_rx.recv().await {
            pending -= 1;

            let mut board: Chess = res.fen.clone().into_position(CastlingMode::Standard)?;
            let mov = res.mov.to_move(&board)?;
            let score = match board.turn() {
                Color::White => res.eval,
                Color::Black => res.eval.rev(),
            };

            info!(
                fen = %res.fen,
                mov = %res.mov,
                eval = ?score,
                pending,
                total = total_moves,
                "Move analysed"
            );

            board.play_unchecked(&mov);

            if !board.is_game_over() {
                let fen = Fen::from_position(board, EnPassantMode::Always);
                fen_tx.send(fen).await?;
                total_moves += 1;
                pending += 1;
            }

            if pending == 0 {
                break;
            }
        }

        drop(fen_tx);
        let engine = task.await??;

        info!("Engine completed");
        engine.quit().await
    }
}

/// Engine analysis details
#[derive(Debug)]
struct EngineAnalysis {
    /// Analysed position
    fen: Fen,
    /// Choosen move
    mov: UciMove,
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

    fn spawn(mut self) -> Result<(EngineTask, Sender<Fen>, Receiver<EngineAnalysis>)> {
        let (fen_tx, mut fen_rx) = mpsc::channel::<Fen>(10);
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

    #[instrument(skip_all, fields(engine=%self.name, fen=%fen), err)]
    async fn process(&mut self, fen: Fen) -> Result<(UciMove, Score)> {
        let mut stream = self.engine.go(fen, self.depth, self.time).await?;

        let mut mov = None;
        let mut eval = None;

        while let Some(info) = stream.info().await? {
            mov = info.line.into_iter().next().or(mov);
            eval = Some(info.score);
        }

        let mov = mov.ok_or_eyre("No move after analysis")?;
        let eval = eval.ok_or_eyre("No eval after analyis")?;

        debug!(%mov, %eval, "Position processed");
        Ok((mov, eval))
    }

    #[instrument(skip_all, fields(engine = %self.name), err)]
    async fn quit(self) -> Result<()> {
        self.engine.quit().await
    }
}
