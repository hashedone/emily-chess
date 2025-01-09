//! Engine possitions processing entities

use std::collections::VecDeque;
use std::fmt::{Debug, Formatter};
use std::time::Duration;

use async_trait::async_trait;
use color_eyre::eyre::OptionExt;
use derivative::Derivative;
use shakmaty::uci::UciMove;
use shakmaty::{Chess, Color, Move, Position};
use tracing::{debug, error, instrument, trace};

use crate::adapters::debug::{DFenExt, FlatOptExt, LineExt, MovExt};
use crate::knowledge::Knowledge;
use crate::uci::Score;
use crate::{config, uci, Result};

use super::processor::{Processor, Scheduled};

/// Engine analysis outcome
#[derive(Derivative)]
#[derivative(Debug)]
pub struct EngineAnalysis {
    /// Analysed variation
    variation: usize,
    /// Halfmoves in variation when analysed
    hm: usize,
    /// Choosen move
    #[derivative(Debug(format_with = "MovExt::fmt"))]
    mov: UciMove,
    /// Engine evaluation
    eval: Score,
}

impl EngineAnalysis {
    /// Creates analysis from engine outcome.
    ///
    /// Note that UCI engines perform analysis in cp from their perspective, our analysis assumes
    /// that eval is always from white perspective - conversion is performed here.
    fn new(variation: usize, hm: usize, fen: Chess, mov: UciMove, eval: Score) -> Self {
        let eval = match fen.turn() {
            Color::White => eval,
            Color::Black => eval.rev(),
        };

        let analysis = Self {
            variation,
            hm,
            mov,
            eval,
        };

        trace!(?analysis, "Engine analysis created");
        analysis
    }
}

impl EngineAnalysis {
    #[instrument(skip(knowledge))]
    fn apply(self, knowledge: &mut Knowledge) -> Result<Scheduled> {
        let (_, position) = knowledge.variation_hm_mut(self.variation, self.hm);
        position.update_eval(self.eval);
        debug!(pos=?position.position().d_fen(), eval=%self.eval, "Applying analysis");

        let mov = self.mov.to_move(position.position())?;
        debug!(mov = ?mov.d_mov(), "Move to schedule");

        let (idx, _, _) = knowledge.add_move(self.variation, self.hm, mov)?;
        knowledge.update_mainline(self.variation, idx);
        let scheduled = Scheduled::new(idx, self.hm + 1);
        trace!(?scheduled, "Move scheduled");

        Ok(scheduled)
    }
}

/// The UCI engine/config wrapper. Not a processor itself, as processor analyses a single
/// game/position list, while single engine instance can be reused. The final processor wuold be a
/// wrapped instance of this.
#[derive(Derivative)]
#[derivative(Debug)]
pub struct Engine {
    engine: uci::Engine,
    #[derivative(Debug(format_with = "FlatOptExt::fmt"))]
    depth: Option<u8>,
    #[derivative(Debug(format_with = "FlatOptExt::fmt"))]
    time: Option<Duration>,
}

impl Engine {
    /// Creates new engine, starts the process
    #[instrument(err)]
    pub async fn new(engine: config::Engine, config: &config::Rev) -> Result<Self> {
        trace!("Creating engine processor");
        let engine = uci::Engine::run(engine).await?;

        Ok(Self {
            engine,
            depth: config.depth,
            time: config.time,
        })
    }

    /// Starts a new game, returns a game processor
    #[instrument(err)]
    pub async fn new_game(&mut self) -> Result<EngineProcessor> {
        trace!("Creating engine processor wrapper");
        self.engine.new_game().await?;
        Ok(EngineProcessor {
            engine: self,
            queue: VecDeque::new(),
            results: vec![],
        })
    }

    /// Gracefully stops the engine
    #[instrument(err)]
    pub async fn quit(self) -> Result<()> {
        self.engine.quit().await
    }

    /// Processes a single variation
    #[instrument(skip(fen, moves), fields(fen=?fen.d_fen(), moves=?moves.d_line()), err)]
    async fn process(&mut self, fen: Chess, moves: Vec<Move>) -> Result<(UciMove, Score)> {
        let mut stream = self
            .engine
            .go(fen.clone(), &moves, self.depth, self.time)
            .await?;

        let mut mov = None;
        let mut eval = None;

        while let Some(info) = stream.info().await? {
            debug!(?mov, ?eval, "Updating best move");
            mov = info.line.into_iter().next().or(mov);
            eval = Some(info.score);
        }

        let mov = mov.ok_or_eyre("No move after analysis")?;
        let eval = eval.ok_or_eyre("No eval after analyis")?;
        debug!(%mov, %eval, "Position processed");

        Ok((mov, eval))
    }
}

#[derive(Derivative, Clone)]
#[derivative(Debug)]
struct Enqueued {
    variation: usize,
    hm: usize,
    #[derivative(Debug(format_with = "DFenExt::fmt"))]
    fen: Chess,
    #[derivative(Debug(format_with = "LineExt::fmt"))]
    moves: Vec<Move>,
}

pub struct EngineProcessor<'a> {
    engine: &'a mut Engine,
    queue: VecDeque<Enqueued>,
    results: Vec<EngineAnalysis>,
}

impl Debug for EngineProcessor<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineProcessor")
            .field("engine", self.engine)
            .finish()
    }
}

#[async_trait]
impl Processor for EngineProcessor<'_> {
    #[instrument(skip(knowledge))]
    fn enqueue(&mut self, knowledge: &mut Knowledge, schedule: &[Scheduled]) {
        let knowledge = &*knowledge;

        let schedule = schedule
            .iter()
            .filter(|scheduled| {
                let (variation, _) = knowledge.variation_hm(scheduled.variation, scheduled.hm);
                variation.moves().len() <= scheduled.hm
            })
            .map(|scheduled| {
                let (variation, position) = knowledge.variation_hm(scheduled.variation, 0);
                let fen = position.position().clone();
                let moves = variation.moves()[..scheduled.hm].to_owned();
                debug!(
                    ?scheduled,
                    fen = ?fen.d_fen(),
                    moves = ?moves.d_line(),
                    "Scheduling variation"
                );

                Enqueued {
                    variation: scheduled.variation,
                    hm: scheduled.hm,
                    fen,
                    moves,
                }
            });

        self.queue.extend(schedule);
        debug!(pending = self.queue.len(), "Scheduling complete");
    }

    #[instrument(skip_all)]
    async fn process(&mut self) {
        let Some(next) = self.queue.pop_front() else {
            trace!("No positions to process");
            return;
        };

        match self.engine.process(next.fen.clone(), next.moves).await {
            Ok((mov, eval)) => {
                let result = EngineAnalysis::new(next.variation, next.hm, next.fen, mov, eval);
                trace!(?result, "New result");
                self.results.push(result);
            }
            Err(err) => error!(%err, "Engine processing failed"),
        }
    }

    #[instrument(skip_all)]
    fn apply_results(&mut self, knowledge: &mut Knowledge) -> Vec<Scheduled> {
        trace!(results = self.results.len(), "Applying results");
        self.results
            .drain(..)
            .filter_map(|res| match res.apply(knowledge) {
                Ok(scheduled) => Some(scheduled),
                Err(err) => {
                    error!(%err, "While applying result to knowledge");
                    None
                }
            })
            .collect()
    }

    fn is_idle(&self) -> bool {
        self.queue.is_empty()
    }
}
