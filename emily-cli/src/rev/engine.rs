//! Engine possitions processing entities

use std::time::Duration;

use async_trait::async_trait;
use color_eyre::eyre::OptionExt;
use shakmaty::{Chess, Color, Move, Position};
use tracing::{debug, info, instrument};

use crate::adapters::TracingAdapt;
use crate::knowledge::Knowledge;
use crate::uci::Score;
use crate::{config, uci, Result};

use super::processor::{BoxedResult, ProcessingResult, Processor};

/// Engine analysis outcome
#[derive(Debug)]
pub struct EngineAnalysis {
    /// Analysed position
    fen: Chess,
    /// Choosen move
    mov: Move,
    /// Engine evaluation
    eval: Score,
}

impl EngineAnalysis {
    /// Creates analysis from engine outcome.
    ///
    /// Note that UCI engines perform analysis in cp from their perspective, our analysis assumes
    /// that eval is always from white perspective - conversion is performed here.
    fn new(fen: Chess, mov: Move, eval: Score) -> Self {
        let eval = match fen.turn() {
            Color::White => eval,
            Color::Black => eval.rev(),
        };

        Self { fen, mov, eval }
    }
}

impl ProcessingResult for EngineAnalysis {
    #[instrument(skip_all, fields(fen=%self.fen.tr(), mov=%self.mov, eval=%self.eval))]
    fn apply(&self, knowledge: &mut Knowledge) -> Result<Vec<Chess>> {
        info!(eval=%self.eval, "Move analysed");
        knowledge.pos_mut(self.fen.clone()).update_eval(self.eval);

        let next_fen = knowledge
            .mov_mut(self.fen.clone(), self.mov.clone())?
            .position()
            .clone();
        Ok(vec![next_fen])
    }
}

/// The UCI engine/config wrapper. Not a processor itself, as processor analyses a single
/// game/position list, while single engine instance can be reused. The final processor wuold be a
/// wrapped instance of this.
pub struct Engine {
    engine: uci::Engine,
    name: String,
    depth: Option<u8>,
    time: Option<Duration>,
}

impl Engine {
    /// Creates new engine, starts the process
    #[instrument(err)]
    pub async fn new(engine: config::Engine, config: &config::Rev) -> Result<Self> {
        let name = engine.name.clone();
        let engine = uci::Engine::run(engine).await?;

        Ok(Self {
            engine,
            name,
            depth: config.depth,
            time: config.time,
        })
    }

    /// Starts a new game, returns a game processor
    #[instrument(skip_all, fields(engine=%self.name), err)]
    pub async fn new_game(&mut self) -> Result<EngineProcessor> {
        self.engine.new_game().await?;
        Ok(EngineProcessor { engine: self })
    }

    /// Gracefully stops the engine
    #[instrument(skip_all, fields(engine = %self.name), err)]
    pub async fn quit(self) -> Result<()> {
        self.engine.quit().await
    }

    #[instrument(skip_all, fields(engine=%self.name, fen=fen.tr()), err)]
    async fn process(&mut self, fen: Chess) -> Result<EngineAnalysis> {
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

        let res = EngineAnalysis::new(fen, mov, eval);
        Ok(res)
    }
}

pub struct EngineProcessor<'a> {
    engine: &'a mut Engine,
}

#[async_trait]
impl Processor for EngineProcessor<'_> {
    #[instrument(skip_all, fields(fen=%fen.tr()), err)]
    fn should_process(&mut self, knowledge: &mut Knowledge, fen: &Chess) -> Result<bool> {
        Ok(knowledge.pos_mut(fen.clone()).eval().is_none())
    }

    #[instrument(skip_all, fields(fen=%fen.tr()))]
    async fn process(&mut self, fen: Chess) -> Result<BoxedResult> {
        let res = self.engine.process(fen).await?;
        Ok(Box::new(res) as _)
    }
}
