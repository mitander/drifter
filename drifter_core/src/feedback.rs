use crate::corpus::Corpus;
use crate::input::Input;
use md5;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Length of an MD5 digest in bytes. Used for sizing hash arrays.
const MD5_DIGEST_LEN: usize = 16;

/// Defines errors that can occur within `Feedback` mechanisms.
#[derive(Error, Debug)]
pub enum FeedbackError {
    /// Error during the initialization phase of the feedback component.
    #[error("Feedback initialization failed: {0}")]
    InitializationFailed(String),
    /// An error occurred while the feedback component was interacting with the `Corpus`.
    #[error("Feedback interaction with corpus failed: {0}")]
    CorpusInteraction(#[from] crate::corpus::CorpusError),
    /// Data from a targeted `Observer` was not found, was `None` when `Some` was expected,
    /// or was in an unexpected format (e.g., wrong length).
    #[error("Observer data error for observer '{observer_name}': {reason}")]
    ObserverData {
        observer_name: String,
        reason: String,
    },
}

/// A `Feedback` mechanism evaluates data from `Observer`s to determine if an
/// execution (and its corresponding `Input`) is "interesting."
///
/// If deemed interesting, the `Feedback` component is also responsible for (or delegating)
/// the addition of this new interesting input to the `Corpus`.
pub trait Feedback<I: Input>: Send + Sync {
    /// Returns a static string name identifying the feedback mechanism.
    /// Useful for logging and debugging.
    fn name(&self) -> &'static str;

    /// Initializes the feedback mechanism, potentially using the initial state of the `Corpus`.
    ///
    /// For example, a coverage feedback might process all initial seeds in the corpus
    /// to establish a baseline coverage map.
    ///
    /// # Arguments
    /// * `corpus`: A reference to the `Corpus`.
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(FeedbackError)` if initialization fails.
    fn init(&mut self, corpus: &dyn Corpus<I>) -> Result<(), FeedbackError>;

    /// Determines if the outcome of executing an `Input` is "interesting."
    ///
    /// "Interestingness" is defined by the specific feedback implementation. It usually
    /// means the input led to a new state (e.g., new code coverage) or desirable outcome.
    ///
    /// # Arguments
    /// * `input`: The `Input` that was executed. (May not be used by all feedbacks directly).
    /// * `observers_data`: A map where keys are observer names (matching `Observer::name()`)
    ///   and values are `Option<Vec<u8>>` representing their serialized data from the last execution.
    /// * `corpus`: A reference to the `Corpus` (e.g., to check against existing entries or properties).
    ///
    /// # Returns
    /// `Ok(true)` if the input is considered interesting, `Ok(false)` otherwise.
    /// `Err(FeedbackError)` if an error occurs during the evaluation.
    fn is_interesting(
        &mut self,
        input: &I,
        observers_data: &HashMap<&'static str, Option<Vec<u8>>>,
        corpus: &dyn Corpus<I>,
    ) -> Result<bool, FeedbackError>;

    /// Adds an `Input` (which was presumably deemed interesting) to the `Corpus`.
    ///
    /// The method should handle the creation of appropriate metadata for the corpus entry.
    ///
    /// # Arguments
    /// * `input_to_add`: The `Input` to be added. Ownership is taken.
    /// * `corpus`: A mutable reference to the `Corpus` where the input will be added.
    ///
    /// # Returns
    /// `Ok(true)` if the input was successfully added to the corpus.
    /// `Ok(false)` if the input was, for example, deemed a duplicate by this feedback
    /// mechanism at the point of addition and thus not added.
    /// `Err(FeedbackError)` if an error occurs during addition (e.g., corpus I/O error).
    fn add_to_corpus(
        &mut self,
        input_to_add: I,
        corpus: &mut dyn Corpus<I>,
    ) -> Result<bool, FeedbackError>;

    /// Returns the feedback component as a `&dyn Any` reference for downcasting.
    fn as_any(&self) -> &dyn Any;
    /// Returns the feedback component as a mutable `&mut dyn Any` reference for downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// A mock feedback mechanism that tracks "coverage" based on input hashes.
///
/// It uses data from a specified `Observer` (assumed to be `MockCoverageObserver` or similar,
/// which provides an MD5 hash as its data). An input is considered interesting if this
/// observed hash has not been seen before.
#[derive(Debug)]
pub struct MockBitmapCoverageFeedback {
    /// Set of MD5 hashes representing "coverage" encountered so far.
    pub global_coverage_hashes: HashSet<[u8; MD5_DIGEST_LEN]>,
    /// The `name()` of the `Observer` this feedback should get data from.
    target_observer_name: String,
}

impl MockBitmapCoverageFeedback {
    /// Creates a new `MockBitmapCoverageFeedback`.
    ///
    /// # Arguments
    /// * `target_observer_name`: The name of the `Observer` (e.g., "MockCoverageObserver")
    ///   whose serialized data (expected to be an MD5 hash)
    ///   this feedback will use to determine interestingness.
    pub fn new(target_observer_name: String) -> Self {
        Self {
            global_coverage_hashes: HashSet::new(),
            target_observer_name,
        }
    }
}

impl<I: Input> Feedback<I> for MockBitmapCoverageFeedback {
    fn name(&self) -> &'static str {
        "MockBitmapCoverageFeedback"
    }

    fn init(&mut self, _corpus: &dyn Corpus<I>) -> Result<(), FeedbackError> {
        Ok(())
    }

    fn is_interesting(
        &mut self,
        _input: &I,
        observers_data: &HashMap<&'static str, Option<Vec<u8>>>,
        _corpus: &dyn Corpus<I>,
    ) -> Result<bool, FeedbackError> {
        match observers_data.get(self.target_observer_name.as_str()) {
            Some(Some(coverage_data_bytes)) => {
                if coverage_data_bytes.len() == MD5_DIGEST_LEN {
                    let mut hash_array = [0u8; MD5_DIGEST_LEN];
                    hash_array.copy_from_slice(coverage_data_bytes);
                    Ok(!self.global_coverage_hashes.contains(&hash_array))
                } else {
                    Err(FeedbackError::ObserverData {
                        observer_name: self.target_observer_name.clone(),
                        reason: format!(
                            "Expected {} bytes of coverage data, but got {}",
                            MD5_DIGEST_LEN,
                            coverage_data_bytes.len()
                        ),
                    })
                }
            }
            Some(None) => Ok(false),
            None => Err(FeedbackError::ObserverData {
                observer_name: self.target_observer_name.clone(),
                reason: "Required observer data not found in observers_data map.".to_string(),
            }),
        }
    }

    /// Adds the `input_to_add` to the `corpus`.
    ///
    /// This implementation assumes that if `is_interesting` was true, it was due to
    /// the `target_observer_name` providing the MD5 hash of `input_to_add`.
    /// It re-calculates this hash from `input_to_add` to store it.
    fn add_to_corpus(
        &mut self,
        input_to_add: I,
        corpus: &mut dyn Corpus<I>,
    ) -> Result<bool, FeedbackError> {
        // Re-calculate hash from input_to_add, assuming this matches what the observer provided.
        let input_hash = md5::compute(input_to_add.as_bytes()).0;

        if self.global_coverage_hashes.insert(input_hash) {
            let metadata_description = format!(
                "Added by MockBitmapCoverageFeedback, new coverage hash: {:x}",
                md5::Digest(input_hash) // md5::Digest can be formatted directly
            );
            let metadata: Box<dyn Any + Send + Sync> = Box::new(metadata_description);

            corpus.add(input_to_add, metadata)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A `Feedback` mechanism that considers an input interesting if its content (MD5 hash)
/// has not been seen before.
///
/// This feedback directly hashes the input itself and does not rely on data from other observers.
#[derive(Default, Debug)]
pub struct UniqueInputFeedback {
    /// Set of MD5 hashes of inputs already known to this feedback mechanism.
    known_input_hashes: HashSet<[u8; MD5_DIGEST_LEN]>,
}

impl UniqueInputFeedback {
    /// Creates a new `UniqueInputFeedback` with an empty set of known hashes.
    pub fn new() -> Self {
        Default::default()
    }

    /// Adds the MD5 hash of a given `Input` to the set of known hashes.
    ///
    /// This method can be used to pre-populate the feedback with hashes from an
    /// initial seed corpus, so that original seeds are not re-evaluated as "new"
    /// by this feedback during the fuzzing loop.
    /// This method is not part of the `Feedback` trait and must be called explicitly if needed.
    pub fn add_known_input_hash(&mut self, input: &impl Input) {
        let hash = md5::compute(input.as_bytes()).0;
        self.known_input_hashes.insert(hash);
    }
}

impl<I: Input> Feedback<I> for UniqueInputFeedback {
    fn name(&self) -> &'static str {
        "UniqueInputFeedback"
    }

    fn init(&mut self, _corpus: &dyn Corpus<I>) -> Result<(), FeedbackError> {
        // Could iterate `_corpus` and call `add_known_input_hash` for each entry
        // if the fuzzer setup loop doesn't handle initial seed registration with feedbacks.
        Ok(())
    }

    /// Determines if the `input` is interesting by checking if its MD5 hash is new.
    /// Ignores `observers_data` and `corpus` arguments for this simple check.
    fn is_interesting(
        &mut self,
        input: &I,
        _observers_data: &HashMap<&'static str, Option<Vec<u8>>>,
        _corpus: &dyn Corpus<I>,
    ) -> Result<bool, FeedbackError> {
        let hash = md5::compute(input.as_bytes()).0;
        Ok(!self.known_input_hashes.contains(&hash))
    }

    /// Adds the `input_to_add` to the `corpus` if its MD5 hash is new.
    fn add_to_corpus(
        &mut self,
        input_to_add: I,
        corpus: &mut dyn Corpus<I>,
    ) -> Result<bool, FeedbackError> {
        let hash = md5::compute(input_to_add.as_bytes()).0;
        if self.known_input_hashes.insert(hash) {
            let metadata_description = format!(
                "Added by UniqueInputFeedback, unique hash: {:x}",
                md5::Digest(hash)
            );
            let metadata: Box<dyn Any + Send + Sync> = Box::new(metadata_description);
            corpus.add(input_to_add, metadata)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::InMemoryCorpus;

    #[test]
    fn unique_input_feedback_identifies_adds_and_skips_inputs_correctly() {
        let mut feedback = UniqueInputFeedback::new();
        let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
        let observers_data = HashMap::new();

        assert!(
            feedback.init(&corpus).is_ok(),
            "Init should succeed without error"
        );

        let input1: Vec<u8> = vec![1, 2, 3, 4, 5];
        assert!(
            feedback
                .is_interesting(&input1, &observers_data, &corpus)
                .unwrap(),
            "Input1 should be interesting as it's new"
        );

        let added1 = feedback.add_to_corpus(input1.clone(), &mut corpus).unwrap();
        assert!(added1, "Input1 should be successfully added to the corpus");
        assert_eq!(
            corpus.len(),
            1,
            "Corpus length should be 1 after adding input1"
        );
        assert!(
            feedback
                .known_input_hashes
                .contains(&md5::compute(input1.as_bytes()).0),
            "Input1's hash should now be known"
        );

        assert!(
            !feedback
                .is_interesting(&input1, &observers_data, &corpus)
                .unwrap(),
            "Input1 should NOT be interesting again as its hash is known"
        );
        let added1_again = feedback.add_to_corpus(input1.clone(), &mut corpus).unwrap();
        assert!(
            !added1_again,
            "Input1 should NOT be added to the corpus again"
        );
        assert_eq!(corpus.len(), 1, "Corpus length should remain 1");

        let input2: Vec<u8> = vec![6, 7, 8];
        assert!(
            feedback
                .is_interesting(&input2, &observers_data, &corpus)
                .unwrap(),
            "Input2 (different content) should be interesting"
        );
        let added2 = feedback.add_to_corpus(input2.clone(), &mut corpus).unwrap();
        assert!(added2, "Input2 should be successfully added to the corpus");
        assert_eq!(
            corpus.len(),
            2,
            "Corpus length should be 2 after adding input2"
        );
        assert!(
            feedback
                .known_input_hashes
                .contains(&md5::compute(input2.as_bytes()).0),
            "Input2's hash should now be known"
        );
    }

    #[test]
    fn unique_input_feedback_respects_pre_added_known_hashes() {
        let mut feedback = UniqueInputFeedback::new();
        let input1: Vec<u8> = vec![10, 20, 30];
        let observers_data = HashMap::new();
        let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();

        // Pre-populate a hash
        feedback.add_known_input_hash(&input1);

        assert!(
            !feedback
                .is_interesting(&input1, &observers_data, &corpus)
                .unwrap(),
            "Input1 should NOT be interesting after its hash was added via add_known_input_hash"
        );
        assert!(
            !feedback.add_to_corpus(input1, &mut corpus).unwrap(),
            "Input1 should not be added if its hash is pre-known"
        );
        assert_eq!(corpus.len(), 0, "Corpus should remain empty");
    }

    #[test]
    fn mock_bitmap_coverage_feedback_evaluates_and_adds_based_on_observer_data() {
        const MOCK_OBSERVER_NAME: &str = "MyMockCoverageObserver";
        let mut feedback = MockBitmapCoverageFeedback::new(MOCK_OBSERVER_NAME.to_string());
        let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();

        assert!(feedback.init(&corpus).is_ok(), "Init should succeed");

        let input1: Vec<u8> = vec![11, 22, 33];
        let input1_hash_array = md5::compute(input1.as_bytes()).0;

        let mut observers_data_for_input1: HashMap<&'static str, Option<Vec<u8>>> = HashMap::new();
        observers_data_for_input1.insert(MOCK_OBSERVER_NAME, Some(input1_hash_array.to_vec()));

        assert!(
            feedback
                .is_interesting(&input1, &observers_data_for_input1, &corpus)
                .unwrap(),
            "Input1 should be interesting as its observer-provided hash is new"
        );

        let added1 = feedback.add_to_corpus(input1.clone(), &mut corpus).unwrap();
        assert!(added1, "Input1 should be added to corpus");
        assert_eq!(
            corpus.len(),
            1,
            "Corpus size should be 1 after adding input1"
        );
        assert!(
            feedback.global_coverage_hashes.contains(&input1_hash_array),
            "Input1's hash should be stored in global_coverage_hashes"
        );

        assert!(
            !feedback
                .is_interesting(&input1, &observers_data_for_input1, &corpus)
                .unwrap(),
            "Input1 should NOT be interesting again as its hash is now known via global_coverage_hashes"
        );

        let input2: Vec<u8> = vec![44, 55];
        let input2_hash_array = md5::compute(input2.as_bytes()).0;
        let mut observers_data_for_input2: HashMap<&'static str, Option<Vec<u8>>> = HashMap::new();
        observers_data_for_input2.insert(MOCK_OBSERVER_NAME, Some(input2_hash_array.to_vec()));

        assert!(
            feedback
                .is_interesting(&input2, &observers_data_for_input2, &corpus)
                .unwrap(),
            "Input2 should be interesting with its new hash"
        );
        let added2 = feedback.add_to_corpus(input2.clone(), &mut corpus).unwrap();
        assert!(added2, "Input2 should be added to corpus");
        assert_eq!(corpus.len(), 2, "Corpus size should be 2");
        assert!(
            feedback.global_coverage_hashes.contains(&input2_hash_array),
            "Input2's hash should be stored"
        );

        let mut wrong_len_observer_data: HashMap<&'static str, Option<Vec<u8>>> = HashMap::new();
        wrong_len_observer_data.insert(MOCK_OBSERVER_NAME, Some(vec![1, 2, 3]));
        match feedback.is_interesting(&input1, &wrong_len_observer_data, &corpus) {
            Err(FeedbackError::ObserverData {
                observer_name,
                reason,
            }) => {
                assert_eq!(
                    observer_name, MOCK_OBSERVER_NAME,
                    "Error should specify the correct observer name"
                );
                assert!(
                    reason.contains("Expected 16 bytes"),
                    "Error reason should mention expected length"
                );
            }
            other_result => panic!(
                "Expected ObserverDataError for wrong data length, got {:?}",
                other_result
            ),
        }

        let mut missing_observer_data: HashMap<&'static str, Option<Vec<u8>>> = HashMap::new();
        missing_observer_data.insert("SomeOtherObserverName", Some(input1_hash_array.to_vec()));
        match feedback.is_interesting(&input1, &missing_observer_data, &corpus) {
            Err(FeedbackError::ObserverData {
                observer_name,
                reason,
            }) => {
                assert_eq!(
                    observer_name, MOCK_OBSERVER_NAME,
                    "Error should specify the targeted observer name"
                );
                assert!(
                    reason.contains("not found in observers_data map"),
                    "Error reason should indicate data not found"
                );
            }
            other_result => panic!(
                "Expected ObserverDataError for missing observer data, got {:?}",
                other_result
            ),
        }

        let mut none_observer_data: HashMap<&'static str, Option<Vec<u8>>> = HashMap::new();
        none_observer_data.insert(MOCK_OBSERVER_NAME, None);
        assert!(
            !feedback
                .is_interesting(&input1, &none_observer_data, &corpus)
                .unwrap(),
            "Should not be interesting if observer provides None data"
        );
    }
}
