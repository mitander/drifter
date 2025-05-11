use crate::corpus::Corpus;
use crate::input::Input;
use rand_core::RngCore;
use std::any::Any;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SchedulerError {
    #[error("Corpus is empty, cannot schedule next input")]
    CorpusEmpty,
    #[error("Corpus interaction failed within scheduler: {0}")]
    CorpusInteractionError(#[from] crate::corpus::CorpusError),
}

pub trait Scheduler<I: Input>: Send + Sync {
    fn next(
        &mut self,
        corpus: &dyn Corpus<I>,
        rng: &mut dyn RngCore,
    ) -> Result<usize, SchedulerError>;
    fn report_feedback(&mut self, input_id: usize, feedback_value: &dyn Any, is_solution: bool);
}

#[derive(Default)]
pub struct RandomScheduler;

impl RandomScheduler {
    pub fn new() -> Self {
        Self
    }
}

impl<I: Input> Scheduler<I> for RandomScheduler {
    fn next(
        &mut self,
        corpus: &dyn Corpus<I>,
        rng: &mut dyn RngCore,
    ) -> Result<usize, SchedulerError> {
        if corpus.is_empty() {
            return Err(SchedulerError::CorpusEmpty);
        }
        match corpus.random_select(rng) {
            Some((id, _, _)) => Ok(id),
            None => Err(SchedulerError::CorpusEmpty),
        }
    }

    fn report_feedback(&mut self, _input_id: usize, _feedback_value: &dyn Any, _is_solution: bool) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{Corpus, InMemoryCorpus};
    use rand_chacha::ChaCha8Rng;
    use rand_core::SeedableRng;
    use std::any::Any;

    #[test]
    fn random_scheduler_next() {
        let mut scheduler = RandomScheduler::new();
        let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
        let mut rng = ChaCha8Rng::from_seed([0; 32]);

        match scheduler.next(&corpus, &mut rng) {
            Err(SchedulerError::CorpusEmpty) => {}
            Err(e) => panic!("Expected CorpusEmpty error, got {e:?}"),
            Ok(_) => panic!("Expected error for empty corpus, got Ok"),
        }

        let meta1: Box<dyn Any + Send + Sync> = Box::new(());
        corpus.add(vec![1], meta1).unwrap();

        let meta2: Box<dyn Any + Send + Sync> = Box::new(());
        corpus.add(vec![2], meta2).unwrap();

        let mut selected_count = 0;
        for _ in 0..10 {
            if scheduler.next(&corpus, &mut rng).is_ok() {
                selected_count += 1;
            }
        }
        assert_eq!(selected_count, 10);
    }

    #[test]
    fn random_scheduler_report_feedback_is_noop() {
        let mut scheduler: RandomScheduler = RandomScheduler::new();
        <RandomScheduler as Scheduler<Vec<u8>>>::report_feedback(&mut scheduler, 0, &(), false);
    }
}
