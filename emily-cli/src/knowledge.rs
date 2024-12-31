//! The knowledge about positions we gathered for a single game/analysis

use std::collections::HashMap;

use shakmaty::fen::Fen;
use shakmaty::san::San;

#[allow(unused)]
#[derive(Debug)]
pub struct Knowledge {
    /// Game starting position
    root: Fen,
    /// Information per position
    data: HashMap<Fen, Entry>,
}

impl Knowledge {
    pub fn new(root: Fen) -> Self {
        let mut data = HashMap::new();
        data.insert(
            root.clone(),
            Entry {
                history: vec![],
                moves: vec![],
            },
        );

        Self { root, data }
    }
}

/// Single position description
#[allow(unused)]
#[derive(Debug)]
struct Entry {
    /// Moves that was played to achieve this position for the first time (for the purpose of
    /// transposition handling)
    history: Vec<San>,
    /// Moves we consider from this position, and the fen after the move
    moves: Vec<(San, Fen)>,
}
