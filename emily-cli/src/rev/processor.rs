//! The traits for a position processors

use async_trait::async_trait;

use crate::knowledge::Knowledge;

/// Variation to be scheduled for processing
#[derive(Debug, Clone)]
pub struct Scheduled {
    /// Variation to add a move to
    pub variation: usize,
    /// Halfmoves where to process variation
    pub hm: usize,
}

impl Scheduled {
    pub fn new(variation: usize, hm: usize) -> Self {
        Self { variation, hm }
    }
}

/// Entity processing prositions
#[async_trait]
pub trait Processor {
    /// Adds variations to be processed to internal queue. It is allowed to skip transactions that
    /// are not relevant or already processed. If the variation can be processed in-place (without
    /// blocking), it can also be processed immediately instead of enqueing.
    ///
    /// Passed variations are never the concluded games (ie. `Variation::outcome` is always
    /// `None`).
    fn enqueue(&mut self, knowledge: &mut Knowledge, schedule: &[Scheduled]);

    /// Processes single position stored internally. The processing result should also be stored
    /// and will be applied later.
    async fn process(&mut self);

    /// Applies results accumulated so far. Returns moves to further analyse
    fn apply_results(&mut self, knowledge: &mut Knowledge) -> Vec<Scheduled>;

    /// Returns if the processor has work to do
    fn is_idle(&self) -> bool;
}
