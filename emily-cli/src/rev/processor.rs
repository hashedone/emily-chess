//! The traits for a position processors

use async_trait::async_trait;
use shakmaty::Chess;

use crate::knowledge::Knowledge;
use crate::Result;

pub type BoxedResult = Box<dyn ProcessingResult + 'static>;

/// Trait for anything that can be returned as processing result.
pub trait ProcessingResult {
    /// Applies the result to the knowledge base and returns further positions to be analysed
    fn apply(&self, knowledge: &mut Knowledge) -> Result<Vec<Chess>>;
}

/// Entity processing prositions
#[async_trait]
pub trait Processor {
    /// Cheks if the position should be processed.
    fn should_process(&mut self, knowledge: &mut Knowledge, fen: &Chess) -> Result<bool>;

    /// Processes single position
    ///
    /// The knowledge can be used to determine if the move should be processed (eg. if it was
    /// already processed by this processor - the dispatcher would not perform any cycle detection,
    /// but the final positions are never send to be processed) and/or perform a processing
    /// preparation on the move.
    ///
    /// The function returns a future that completes when move analysis is ready and returns a
    /// boxed `ProcessorResult` on which the `apply` method would be called to update the
    /// knowledge.
    async fn process(&mut self, fen: Chess) -> Result<BoxedResult>;
}
