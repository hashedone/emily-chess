//! Types helping proper record tracing

use shakmaty::fen::Fen;
use shakmaty::{EnPassantMode, Position};

/// Adapter trait for types that we want to change how are they recorded in tracing.
pub trait TracingAdapt {
    fn tr(&self) -> String;
}

impl<T: Position + Clone> TracingAdapt for T {
    fn tr(&self) -> String {
        let fen = Fen::from_position(self.clone(), EnPassantMode::Always);
        format!("{fen}")
    }
}
