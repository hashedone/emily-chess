//! Dispatches possitions and knowledge update across processors

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tracing::{debug, info, instrument};

use super::processor::{Processor, Scheduled};
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
        Dispatcher {
            processors: self
                .processors
                .into_iter()
                .map(|processor| ProcessorItem {
                    processor,
                    enqueued: 0,
                })
                .collect(),
            schedule: vec![],
        }
    }
}

pub struct Dispatcher<'a> {
    processors: Vec<ProcessorItem<'a>>,
    schedule: Vec<Scheduled>,
}

struct ProcessorItem<'a> {
    processor: Box<dyn Processor + 'a>,
    enqueued: usize,
}

impl ProcessorItem<'_> {
    async fn process(mut self) -> Self {
        self.processor.process().await;
        self
    }
}

impl<'a> Dispatcher<'a> {
    /// Creates new builder
    pub fn builder() -> DispatcherBuilder<'a> {
        DispatcherBuilder::new()
    }

    /// Dispatchess position untill they are produced, finishes when no more positions are
    /// scheduled for analysis
    #[instrument(skip(self, knowledge), err)]
    pub async fn dispatch(
        mut self,
        knowledge: &mut Knowledge,
        variation: usize,
        hm: usize,
    ) -> Result<()> {
        let schedule = &[Scheduled { variation, hm }];
        let mut processing: FuturesUnordered<_> = self
            .processors
            .into_iter()
            .map(|mut item| {
                item.processor.enqueue(knowledge, schedule);
                item.process()
            })
            .collect();
        let mut idle: Vec<ProcessorItem> = Vec::with_capacity(processing.len());

        debug!("Dispatching started");
        while let Some(mut p) = processing.next().await {
            let schedule = p
                .processor
                .apply_results(knowledge)
                .into_iter()
                .filter(|schedule| {
                    let (variation, _) = knowledge.variation_hm(schedule.variation, schedule.hm);
                    variation.moves().len() < hm || variation.outcome().is_none()
                });

            self.schedule.extend(schedule);
            let schedule = &self.schedule[p.enqueued..];

            debug!(total=?self.schedule.len(), "Scheduled new moves moves");
            p.processor.enqueue(knowledge, schedule);
            p.enqueued += schedule.len();

            for mut idl in idle.drain(..) {
                let schedule = &self.schedule[idl.enqueued..];
                idl.processor.enqueue(knowledge, schedule);
                idl.enqueued += schedule.len();

                processing.push(idl.process());
            }

            match p.processor.is_idle() {
                true => idle.push(p),
                false => processing.push(p.process()),
            }
        }

        info!(
            total_analysed = self.schedule.len() + 1,
            "Dispathing finished"
        );

        Ok(())
    }
}
