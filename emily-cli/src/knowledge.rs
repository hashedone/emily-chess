//! The knowledge about positions we gathered for a single game/analysis

use std::collections::HashMap;

use color_eyre::eyre::ensure;
use shakmaty::{Chess, Move, Outcome, Position};
use tracing::{debug, instrument, trace, warn};

use crate::adapters::TracingAdapt;
use crate::uci::Score;
use crate::Result;

use self::pgn::Pgn;

mod pgn;

/// The single variation considered. Variations describes a particular way a position is reached
/// and it is possible for a variation to repeat a position (up to three times after which draw is
/// assumed)
#[derive(Debug, Clone)]
pub struct Variation {
    /// Moves in this variation
    moves: Vec<Move>,
    /// Positions in this variation, including the root position. Note that the position after the
    /// move `idx` is here at the `idx + 1` index.
    positions: Vec<usize>,
    /// Variation outcome (after the last move)
    outcome: Option<Outcome>,
}

impl Variation {
    /// Creates no-moves variations endine on the root.
    pub fn new(outcome: Option<Outcome>) -> Self {
        Self {
            moves: vec![],
            positions: vec![0],
            outcome,
        }
    }

    /// Checks if 3-fold repetition occured. Only the last position can repeat, as after 3-fold
    /// repetition we never expand variation further
    fn repetition(&self) -> bool {
        let lastidx = self.positions.last().unwrap_or(&0);
        self.positions.iter().filter(|idx| *idx == lastidx).count() >= 3
    }

    /// Accesses moves in the variation
    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    /// Variation outcome
    pub fn outcome(&self) -> Option<&Outcome> {
        self.outcome.as_ref()
    }
}

/// Single position details
#[derive(Debug)]
pub struct PosInfo {
    /// Position itself
    pos: Chess,
    /// Moves we consider from this position.
    moves: HashMap<Move, MoveInfo>,
    /// Engine evaluation of the position.
    eval: Option<Score>,
}

impl PosInfo {
    fn new(pos: Chess) -> Self {
        Self {
            pos,
            moves: HashMap::new(),
            eval: None,
        }
    }

    /// Gets the position
    pub fn position(&self) -> &Chess {
        &self.pos
    }

    /// Updates engine evaluation
    pub fn update_eval(&mut self, eval: Score) -> &mut Self {
        self.eval = Some(eval);
        self
    }
}

/// Move after the position details. Sometimes the same position might slightly differ depending on
/// where it was achieved from - such information is stored in this type.
#[derive(Debug)]
pub struct MoveInfo;

/// All we know about the analyzed moves. This type has to be exportable (and importable) from/into
/// PGN.
#[derive(Debug)]
pub struct Knowledge {
    /// Information per position - same position reached the same way are considered the same, even
    /// if there was a repetition, which means this is not suitable for game storage as it would
    /// not work for repeating same position in a game!
    positions: Vec<PosInfo>,
    /// Position index for `positions`. Note, that positions are not considering how the position was
    /// reached or if the position was repeated before.
    index: HashMap<Chess, usize>,
    /// Variations we considered.
    variations: Vec<Variation>,
    /// Main line index
    main: usize,
}

impl Knowledge {
    /// Creates new knowledge base
    #[instrument(skip_all, fields(root = root.tr()))]
    pub fn new(root: Chess) -> Self {
        debug!("Creating knowledge base");
        Self {
            // Indexing `root` position as first position occuring
            index: std::iter::once((root.clone(), 0)).collect(),
            positions: vec![PosInfo::new(root.clone())],
            variations: vec![Variation::new(root.outcome())],
            main: 0,
        }
    }

    /// Accesses the variation returning it paired with position info after it is reached. The
    /// position information is returned as mutable reference, so it can be immmediately updated.
    #[instrument(skip(self))]
    pub fn variation_mut(&mut self, idx: usize) -> (&Variation, &mut PosInfo) {
        trace!("Accessing variation");
        let variation = &self.variations[idx];
        let lastposidx = *variation.positions.last().unwrap_or(&0);
        let lastpos = &mut self.positions[lastposidx];

        (variation, lastpos)
    }

    /// Accesses the variation and the position information after `hm` halfmoves.
    #[instrument(skip(self))]
    pub fn variation_hm(&self, idx: usize, hm: usize) -> (&Variation, &PosInfo) {
        trace!("Accessing variation");
        let variation = &self.variations[idx];
        let posidx = variation.positions[hm];
        let pos = &self.positions[posidx];

        (variation, pos)
    }

    /// Accesses the variation and the position information after `hm` halfmoves.
    #[instrument(skip(self))]
    pub fn variation_hm_mut(&mut self, idx: usize, hm: usize) -> (&Variation, &mut PosInfo) {
        trace!("Accessing variation");
        let variation = &self.variations[idx];
        let posidx = variation.positions[hm];
        let pos = &mut self.positions[posidx];

        (variation, pos)
    }

    /// Accesses the main line, just like `Knowledge::variation_mut` but additionally returning the
    /// index to the main line variation.
    #[instrument(skip_all)]
    pub fn main_variation_mut(&mut self) -> (usize, &Variation, &mut PosInfo) {
        trace!("Accessing variation");
        let (variation, lastpos) = self.variation_mut(0);
        (0, variation, lastpos)
    }

    /// Adds new move to the variation after `hm` halfmoves played after the root position. If
    /// that was the last move in this variation, the variation would be extended. If the variation
    /// was already extended, the new branched variation would be crated.
    ///
    /// Anyway the index of a new variation would be returned, as well as reference to the new
    /// variation and mutable position info after the move played. Note, that it doesn't have to be
    /// after the whole variation (for example if variation was already extended with this move,
    /// and more moves after that).
    #[instrument(skip(self), err)]
    pub fn add_move(
        &mut self,
        vidx: usize,
        hm: usize,
        mov: Move,
    ) -> Result<(usize, &Variation, &mut PosInfo)> {
        trace!("Adding move");

        let variation = &self.variations[vidx];
        ensure!(
            variation.moves.len() >= hm,
            "Extending variation after its last move"
        );

        if variation.moves.get(hm + 1) == Some(&mov) {
            // This variation includes move that is being added
            debug!("Adding a move already included in this variation");
            let variation = &self.variations[vidx];
            let posidx = variation.positions[hm + 2];
            let position = &mut self.positions[posidx];

            return Ok((vidx, variation, position));
        }

        ensure!(
            variation.moves.len() > hm || self.variations[vidx].outcome.is_none(),
            "Extending variation beyond game conclusion"
        );

        // Move is to be added - we need to calculate position after it
        let beforeidx = variation.positions[hm];
        let beforefen = self.positions[beforeidx].pos.clone();
        let afterfen = beforefen.play(&mov)?;
        let outcome = afterfen.outcome();

        // Look for a position in index. If position did not occur, we will add new position to the
        // index. The newly created position would always be added to the end of the list.
        let afteridx = *self
            .index
            .entry(afterfen.clone())
            .or_insert(self.positions.len());

        if afteridx == self.positions.len() {
            // Adding new position info
            self.positions.push(PosInfo::new(afterfen));
        }

        let vidx = match variation.moves.len() == hm {
            // Adding move to existing variation
            true => vidx,
            false => {
                // Branching variation
                debug!(newidx = self.variations.len(), "Branching variation");

                let moves = variation.moves[..hm].to_owned();
                let positions = variation.positions[..=hm].to_owned();
                let variation = Variation {
                    moves,
                    positions,
                    outcome: None,
                };

                self.variations.push(variation);
                self.variations.len() - 1
            }
        };

        // Extending variation
        let variation = &mut self.variations[vidx];
        variation.moves.push(mov);
        variation.positions.push(afteridx);

        let outcome = match outcome {
            Some(outcome) => Some(outcome),
            None if variation.repetition() => Some(Outcome::Draw),
            None => None,
        };
        variation.outcome = outcome;

        let variation = &self.variations[vidx];
        let position = &mut self.positions[afteridx];

        Ok((vidx, variation, position))
    }

    /// Updates mainline if `from` is mainline right now
    #[instrument(skip(self))]
    pub fn update_mainline(&mut self, from: usize, to: usize) {
        if from == self.main && to < self.variations.len() {
            debug!("Updating mainline");
            self.main = to;
        }
    }

    /// Acceses position by its index
    pub fn position(&self, pidx: usize) -> &PosInfo {
        &self.positions[pidx]
    }

    /// Acceses position by its index
    pub fn position_mut(&mut self, pidx: usize) -> &mut PosInfo {
        &mut self.positions[pidx]
    }

    /// Accesses the given move details on a position if available
    pub fn movinfo(&self, pidx: usize, mov: &Move) -> Option<&MoveInfo> {
        self.positions[pidx].moves.get(mov)
    }

    /// Return root `PosInfo`
    pub fn root(&self) -> &PosInfo {
        let mainline = &self.variations[self.main];
        &self.positions[mainline.positions[0]]
    }

    /// Retrieves PGN representation for storage
    pub fn pgn(&self) -> Pgn {
        Pgn::new(self)
    }
}
