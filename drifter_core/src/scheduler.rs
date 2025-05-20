use crate::corpus::Corpus;
use crate::input::Input;
use rand_core::RngCore;
use std::any::Any;
use thiserror::Error;

/// Errors that can occur during scheduler operations.
#[derive(Error, Debug)]
pub enum SchedulerError {
    /// Indicates that the corpus is empty, and therefore no input can be scheduled.
    #[error("Corpus is empty, cannot schedule next input")]
    CorpusEmpty,
    /// Wraps an error originating from the corpus backend (e.g., I/O error)
    /// encountered during a scheduler operation.
    #[error("Corpus interaction failed within scheduler: {0}")]
    CorpusInteractionError(#[from] crate::corpus::CorpusError),
}

/// A `Scheduler` is responsible for selecting the next input from the `Corpus` to be fuzzed.
///
/// Schedulers can implement various strategies, from simple random selection
/// to more sophisticated algorithms that prioritize inputs based on feedback
/// (e.g., coverage, execution time, etc.).
pub trait Scheduler<I: Input>: Send + Sync {
    /// Selects and returns the ID of the next input to be processed from the corpus.
    ///
    /// # Arguments
    /// * `corpus`: A mutable reference to the `Corpus` from which to select an input.
    ///   It's mutable in case the scheduler needs to update any state
    ///   associated with corpus entries (though `RandomScheduler` does not).
    /// * `rng`: A mutable reference to a random number generator, to be used if the
    ///   scheduler's logic involves randomness.
    ///
    /// # Returns
    /// A `Result` containing the ID (index) of the selected input within the corpus,
    /// or a `SchedulerError` if selection fails (e.g., if the corpus is empty).
    fn next(
        &mut self,
        corpus: &mut dyn Corpus<I>,
        rng: &mut dyn RngCore,
    ) -> Result<usize, SchedulerError>;

    /// Reports feedback received for a previously executed input.
    ///
    /// This method allows the scheduler to potentially adapt its selection strategy
    /// based on the interestingness or outcomes of inputs. For example, a
    /// coverage-guided scheduler might give higher priority to inputs that
    /// discovered new coverage.
    ///
    /// # Arguments
    /// * `input_id`: The ID of the input within the corpus for which feedback is reported.
    /// * `feedback_value`: An arbitrary piece of data representing the feedback. The
    ///   scheduler needs to be designed to understand and utilize this data.
    /// * `is_solution`: A boolean indicating if this input led to finding a bug/solution.
    ///   This can be a strong signal for some schedulers.
    fn report_feedback(&mut self, input_id: usize, feedback_value: &dyn Any, is_solution: bool);
}

/// A basic `Scheduler` that selects inputs randomly from the corpus.
///
/// This scheduler does not use any feedback to prioritize inputs; every selection
/// is uniformly random among the current corpus entries.
#[derive(Default, Debug)]
pub struct RandomScheduler;

impl RandomScheduler {
    /// Creates a new `RandomScheduler`.
    pub fn new() -> Self {
        RandomScheduler
    }
}

impl<I: Input> Scheduler<I> for RandomScheduler {
    /// Selects an input ID randomly from the corpus.
    ///
    /// Returns `Err(SchedulerError::CorpusEmpty)` if the corpus is empty.
    fn next(
        &mut self,
        corpus: &mut dyn Corpus<I>,
        rng: &mut dyn RngCore,
    ) -> Result<usize, SchedulerError> {
        if corpus.is_empty() {
            return Err(SchedulerError::CorpusEmpty);
        }
        // `random_select` itself should handle the empty case, but this is a clear guard.
        match corpus.random_select(rng) {
            Some((id, _input_ref, _metadata_ref)) => Ok(id),
            None => {
                // This path implies corpus became empty between the check and select,
                // or random_select has an issue on a non-empty corpus.
                // Given &mut dyn Corpus, concurrent modification is not expected here.
                // It's safest to return CorpusEmpty.
                Err(SchedulerError::CorpusEmpty)
            }
        }
    }

    /// This implementation is a no-op, as `RandomScheduler` does not utilize feedback.
    fn report_feedback(&mut self, _input_id: usize, _feedback_value: &dyn Any, _is_solution: bool) {
        // RandomScheduler does not adapt based on feedback, so this is a no-op.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::InMemoryCorpus;
    use rand_chacha::ChaCha8Rng;
    use rand_core::SeedableRng;

    #[test]
    fn random_scheduler_next_from_empty_corpus_returns_corpus_empty_error() {
        let mut scheduler = RandomScheduler::new();
        let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
        let mut rng = ChaCha8Rng::from_seed([0; 32]);

        match scheduler.next(&mut corpus, &mut rng) {
            Err(SchedulerError::CorpusEmpty) => { /* This is the expected outcome */ }
            Err(e) => panic!(
                "Expected SchedulerError::CorpusEmpty, but got different error: {:?}. Corpus length: {}",
                e,
                corpus.len()
            ),
            Ok(id) => panic!(
                "Expected an error when scheduling from an empty corpus, but got Ok with id {}",
                id
            ),
        }
    }

    #[test]
    fn random_scheduler_next_from_non_empty_corpus_returns_valid_id() {
        let mut scheduler = RandomScheduler::new();
        let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
        let mut rng = ChaCha8Rng::from_seed([1; 32]);

        let meta1: Box<dyn Any + Send + Sync> = Box::new("meta_data_for_input_1".to_string());
        corpus
            .add(vec![1, 0, 1], meta1)
            .expect("Failed to add input 1 to corpus");

        let meta2: Box<dyn Any + Send + Sync> = Box::new("meta_data_for_input_2".to_string());
        corpus
            .add(vec![2, 1, 2], meta2)
            .expect("Failed to add input 2 to corpus");

        let mut selected_ids_set = std::collections::HashSet::new();
        let number_of_selections = 50;

        for i in 0..number_of_selections {
            match scheduler.next(&mut corpus, &mut rng) {
                Ok(id) => {
                    assert!(
                        id < corpus.len(),
                        "Selected ID {} is out of bounds for corpus of length {}. Iteration: {}",
                        id,
                        corpus.len(),
                        i
                    );
                    selected_ids_set.insert(id);
                }
                Err(e) => panic!("scheduler.next() failed on iteration {}: {:?}", i, e),
            }
        }
        assert_eq!(
            selected_ids_set.len(),
            corpus.len(),
            "Scheduler should have selected all unique items from a small corpus over {} selections. Selected IDs: {:?}",
            number_of_selections,
            selected_ids_set
        );
        assert!(
            selected_ids_set.contains(&0),
            "ID 0 should have been selected at least once"
        );
        assert!(
            selected_ids_set.contains(&1),
            "ID 1 should have been selected at least once"
        );
    }

    #[test]
    fn random_scheduler_report_feedback_method_is_a_noop_and_compiles() {
        let mut scheduler = RandomScheduler::new();
        let dummy_feedback_value: i32 = 42;

        <RandomScheduler as Scheduler<Vec<u8>>>::report_feedback(
            &mut scheduler,
            0,
            &dummy_feedback_value,
            false,
        );
    }
}
