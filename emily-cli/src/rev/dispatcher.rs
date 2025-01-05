//! Dispatches possitions and knowledge update across processors

use std::time::Duration;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use shakmaty::{Chess, Position};
use tokio::select;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::interval;
use tracing::{info, instrument, warn};

use super::processor::{BoxedResult, Processor};
use crate::knowledge::Knowledge;
use crate::Result;

/// Builder for `Dispatcher`
#[derive(Default)]
pub struct DispatcherBuilder<'a> {
    processors: Vec<Box<dyn Processor + 'a>>,
}

impl<'a> DispatcherBuilder<'a> {
    /// Initializes the builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a new processor
    pub fn with(&mut self, processor: impl Processor + 'a) -> &mut Self {
        self.processors.push(Box::new(processor) as _);
        self
    }

    /// Builds a final dispatcher
    pub fn build(self) -> Dispatcher<'a> {
        let (meta, processors) = self
            .processors
            .into_iter()
            .enumerate()
            .map(|(idx, processor)| {
                let (tx, rx) = unbounded_channel();
                let item = ProcessorItem { processor, rx, idx };
                let meta = ProcessorMeta {
                    sender: tx,
                    total: 0,
                    completed: 0,
                };
                (meta, item)
            })
            .unzip();

        Dispatcher { meta, processors }
    }
}

pub struct Dispatcher<'a> {
    meta: Vec<ProcessorMeta>,
    processors: Vec<ProcessorItem<'a>>,
}

struct ProcessorMeta {
    sender: UnboundedSender<Chess>,
    total: usize,
    completed: usize,
}

struct ProcessorItem<'a> {
    processor: Box<dyn Processor + 'a>,
    rx: UnboundedReceiver<Chess>,
    idx: usize,
}

impl ProcessorItem<'_> {
    async fn wait_pos(mut self) -> Option<(Self, Chess)> {
        let pos = self.rx.recv().await?;
        Some((self, pos))
    }

    async fn process(mut self, pos: Chess) -> (Self, Result<BoxedResult>) {
        let res = self.processor.process(pos).await;
        (self, res)
    }
}

impl<'a> Dispatcher<'a> {
    /// Creates new builder
    pub fn builder() -> DispatcherBuilder<'a> {
        DispatcherBuilder::new()
    }

    /// Scheddules positions for analysis in every processor
    fn schedule_pos(meta: &mut [ProcessorMeta], moves: &[Chess]) {
        for mov in moves {
            if mov.outcome().is_none() {
                for m in &mut *meta {
                    match m.sender.send(mov.clone()) {
                        Err(err) => warn!(%err, "Error while sending move for processing"),
                        Ok(()) => {
                            m.total += 1;
                        }
                    }
                }
            }
        }
    }

    /// Returns if no more positions would need processing
    fn all_done(meta: &[ProcessorMeta]) -> bool {
        meta.iter().all(|m| m.total == m.completed)
    }

    /// Dispatchess position untill they are produced, finishes when no more positions are
    /// scheduled for analysis
    #[instrument(skip_all, err)]
    pub async fn dispatch(self, knowledge: &mut Knowledge, root: Chess) -> Result<()> {
        let mut waiting_pos: FuturesUnordered<_> = self
            .processors
            .into_iter()
            .map(ProcessorItem::wait_pos)
            .collect();

        let mut waiting_res = FuturesUnordered::new();
        let mut meta = self.meta;

        let mut heartbeat = interval(Duration::from_secs(10));

        Self::schedule_pos(&mut meta, &[root]);

        while !Self::all_done(&meta) {
            select! {
                Some(Some((mut p, pos))) = waiting_pos.next() => {
                    match p.processor.should_process(knowledge, &pos) {
                        Ok(true) =>
                            waiting_res.push(p.process(pos)),
                        Ok(false) => {
                            meta[p.idx].completed += 1;
                            waiting_pos.push(p.wait_pos());
                        }
                        Err(err) => {
                            warn!(%err, "While processing position");
                            meta[p.idx].completed += 1;
                            waiting_pos.push(p.wait_pos())
                        }
                    }
                }
                Some((p, res)) = waiting_res.next() =>
                {
                    meta[p.idx].completed += 1;

                    match res {
                        Err(err) => warn!(%err, "Error processing position"),
                        Ok(res) => {
                            match res.apply(knowledge) {
                                Ok(moves) => Self::schedule_pos(&mut meta, &moves),
                                Err(err) => {
                                    warn!(%err, "While applying result");
                                    continue;
                                }
                            };
                        }
                    }

                    waiting_pos.push(p.wait_pos())
                }
                _ = heartbeat.tick() => {
                    let total: usize = meta.iter().map(|m| m.total).sum();
                    let completed: usize = meta.iter().map(|m| m.completed).sum();
                    let progress: usize = completed * 100 / total;
                    info!(total, completed, progress, "Dispatch HB");
                }
            }
        }

        let total: usize = meta.iter().map(|m| m.total).sum();
        info!(total_analysed = total, "Dispathing finished");

        Ok(())
    }
}
