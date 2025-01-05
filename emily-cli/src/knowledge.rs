//! The knowledge about positions we gathered for a single game/analysis

use std::collections::HashMap;

use color_eyre::eyre::OptionExt;
use shakmaty::{Chess, Move, Position};
use tracing::{info, instrument, warn};

use crate::adapters::TracingAdapt;
use crate::uci::Score;
use crate::Result;

use self::pgn::Pgn;

mod pgn;

/// All we know about the analyzed moves. This type has to be exportable (and importable) from/into
/// PGN.
#[derive(Debug)]
pub struct Knowledge {
    /// Game starting position
    root: Chess,
    /// Information per position
    data: HashMap<Chess, PosInfo>,
}

impl Knowledge {
    /// Creates new knowledge base
    #[instrument(skip_all, fields(root = root.tr()))]
    pub fn new(root: Chess) -> Self {
        let mut data = HashMap::new();
        data.insert(root.clone(), PosInfo::default());

        info!("Knowledge base created");
        Self { root, data }
    }

    /// Accesses the position in the knowledge base. If the move was not reached before it would be
    /// added, but note that if the variation reaching this position would not be added in the
    /// future, it would not appear in the final PGN.
    #[instrument(skip_all, fields(fen = fen.tr()))]
    pub fn pos_mut(&mut self, fen: Chess) -> &mut PosInfo {
        self.data.entry(fen).or_insert_with(|| {
            warn!("Position never reached before");
            PosInfo::default()
        })
    }

    /// Accesses the move in the knowledge base adding it if it was not considered yet. If the
    /// position was not considered before, it would also be added to the knowledge base. Note,
    /// that if the moves to reach the position form the root would never be addded, the position
    /// would not occur in the final PGN!
    ///
    /// Adding the position would immediately add the position reached to the knowledge.
    ///
    /// Might fail if the fen is not valid, or mov is not valid in given position (however the move
    /// is *not* validated, so passing invalid move might cause creating unexpected position)
    #[instrument(skip_all, fields(fen = ?fen.tr(), %mov, new_pos))]
    pub fn mov_mut(&mut self, fen: Chess, mov: Move) -> Result<&mut MoveInfo> {
        let pos = self.data.entry(fen.clone()).or_insert_with(|| {
            warn!("Position never reached before");
            PosInfo::default()
        });

        if !pos.moves.contains_key(&mov) {
            let new_pos = fen.clone().play(&mov)?;

            let info = MoveInfo::new(new_pos.clone())?;
            tracing::Span::current().record("new_pos", new_pos.tr());

            info!("Adding new move");
            pos.moves.entry(mov.clone()).or_insert(info);

            self.data.entry(new_pos.clone()).or_insert_with(|| {
                info!("Adding new position");
                PosInfo::new()
            });
        };

        self.data
            .get_mut(&fen)
            .ok_or_eyre("Position not found")?
            .moves
            .get_mut(&mov)
            .ok_or_eyre("Move not found")
    }

    /// Retrieves PGN representation for storage
    pub fn pgn(&self) -> Result<Pgn> {
        Pgn::new(self)
    }
}

/// Single position details
#[derive(Debug, Default)]
pub struct PosInfo {
    /// Moves we consider from this position.
    moves: HashMap<Move, MoveInfo>,
    /// Engine evaluation of the position.
    eval: Option<Score>,
}

impl PosInfo {
    /// Creates new position from history and one move played from there
    fn new() -> Self {
        Self::default()
    }

    /// Gets the engine evaluation
    pub fn eval(&self) -> Option<&Score> {
        self.eval.as_ref()
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
pub struct MoveInfo {
    /// Position after the move is played
    pos: Chess,
}

impl MoveInfo {
    /// Creates a new move entry.
    fn new(pos: Chess) -> Result<Self> {
        Ok(Self { pos })
    }

    /// Retrieves position after the move
    pub fn position(&self) -> &Chess {
        &self.pos
    }
}
