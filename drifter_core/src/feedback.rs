use crate::corpus::{Corpus, OnDiskCorpusEntryMetadata};
use crate::input::Input;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FeedbackError {
    #[error("Initialization error: {0}")]
    InitError(String),
    #[error("Corpus operation failed within feedback: {0}")]
    CorpusInteractionError(#[from] crate::corpus::CorpusError),
    #[error("Observer data not found or in unexpected format for observer: {0}")]
    ObserverDataError(String),
}

pub trait Feedback<I: Input>: Send + Sync {
    fn name(&self) -> &'static str;
    fn init(&mut self, corpus: &dyn Corpus<I>) -> Result<(), FeedbackError>;
    fn is_interesting(
        &mut self,
        input: &I,
        observers_data: &HashMap<&'static str, Option<Vec<u8>>>,
        corpus: &dyn Corpus<I>,
    ) -> Result<bool, FeedbackError>;
    fn add_to_corpus(
        &mut self,
        input_to_add: I,
        corpus: &mut dyn Corpus<I>,
    ) -> Result<bool, FeedbackError>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[derive(Default)]
pub struct MockBitmapCoverageFeedback {
    pub global_coverage_hashes: HashSet<[u8; 16]>,
    target_observer_name: String,
}

impl MockBitmapCoverageFeedback {
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
        // Initialize global_coverage_hashes with coverage from initial corpus inputs.
        // This requires being able to "fake" run each initial corpus item through a mock observer.
        // For now, if we assume MockCoverageObserver, we can hash initial corpus inputs.
        // This part is tricky without an actual observer execution setup during init.
        // Alternative: The fuzzing loop can call `is_interesting` and `add_to_corpus` for initial seeds too.
        // For MVP, let's assume it's built up during fuzzing.
        // If initial seeds are processed by the main loop, their hashes will be added.
        println!(
            "MockBitmapCoverageFeedback initialized. Target observer: {}",
            self.target_observer_name
        );
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
                if coverage_data_bytes.len() == 16 {
                    let mut hash_array = [0u8; 16];
                    hash_array.copy_from_slice(coverage_data_bytes);
                    if !self.global_coverage_hashes.contains(&hash_array) {
                        Ok(true)
                    } else {
                        Ok(false)
                    }
                } else {
                    Err(FeedbackError::ObserverDataError(format!(
                        "Expected 16 bytes of coverage data from {}, got {}",
                        self.target_observer_name,
                        coverage_data_bytes.len()
                    )))
                }
            }
            Some(None) => Ok(false),
            None => Err(FeedbackError::ObserverDataError(format!(
                "Target observer '{}' data not found in observers_data map.",
                self.target_observer_name
            ))),
        }
    }

    fn add_to_corpus(
        &mut self,
        input_to_add: I,
        corpus: &mut dyn Corpus<I>,
    ) -> Result<bool, FeedbackError> {
        let temp_mock_hash_from_input = md5::compute(input_to_add.as_bytes()).0;

        if self
            .global_coverage_hashes
            .insert(temp_mock_hash_from_input)
        {
            let metadata: Box<dyn Any + Send + Sync> = Box::new(OnDiskCorpusEntryMetadata {
                source_description: format!(
                    "New mock coverage: {:x}",
                    md5::Digest(temp_mock_hash_from_input)
                ),
            });
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

#[derive(Default)]
pub struct UniqueInputFeedback {
    known_hashes: HashSet<[u8; 16]>,
}

impl UniqueInputFeedback {
    pub fn new() -> Self {
        Self {
            known_hashes: HashSet::new(),
        }
    }

    pub fn add_known_input_hash(&mut self, input: &impl Input) {
        let hash = md5::compute(input.as_bytes());
        self.known_hashes.insert(hash.0);
    }
}

impl<I: Input> Feedback<I> for UniqueInputFeedback {
    fn name(&self) -> &'static str {
        "UniqueInputFeedback"
    }
    fn init(&mut self, _corpus: &dyn Corpus<I>) -> Result<(), FeedbackError> {
        Ok(())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn is_interesting(
        &mut self,
        input: &I,
        _observers_data: &HashMap<&'static str, Option<Vec<u8>>>,
        _corpus: &dyn Corpus<I>,
    ) -> Result<bool, FeedbackError> {
        let hash = md5::compute(input.as_bytes());
        Ok(!self.known_hashes.contains(&hash.0))
    }

    fn add_to_corpus(
        &mut self,
        input_to_add: I,
        corpus: &mut dyn Corpus<I>,
    ) -> Result<bool, FeedbackError> {
        let hash = md5::compute(input_to_add.as_bytes());
        if self.known_hashes.insert(hash.0) {
            // For OnDiskCorpus, we need OnDiskCorpusEntryMetadata
            // For generality, metadata might need to be passed or constructed more flexibly
            let metadata: Box<dyn Any + Send + Sync> = Box::new(OnDiskCorpusEntryMetadata {
                source_description: format!("Unique hash: {:x}", md5::Digest(hash.0)),
            });
            corpus.add(input_to_add, metadata)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::InMemoryCorpus;

    #[test]
    fn unique_input_feedback_works() {
        let mut feedback = UniqueInputFeedback::new();
        let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
        let observers_data = HashMap::new();

        feedback.init(&corpus).unwrap();

        let input1: Vec<u8> = vec![1, 2, 3];
        assert!(
            feedback
                .is_interesting(&input1, &observers_data, &corpus)
                .unwrap()
        );
        let added1 = feedback.add_to_corpus(input1.clone(), &mut corpus).unwrap();
        assert!(added1);
        assert_eq!(corpus.len(), 1);

        assert!(
            !feedback
                .is_interesting(&input1, &observers_data, &corpus)
                .unwrap()
        );
        let added1_again = feedback.add_to_corpus(input1.clone(), &mut corpus).unwrap();
        assert!(!added1_again);
        assert_eq!(corpus.len(), 1);

        let input2: Vec<u8> = vec![4, 5, 6];
        assert!(
            feedback
                .is_interesting(&input2, &observers_data, &corpus)
                .unwrap()
        );
        let added2 = feedback.add_to_corpus(input2.clone(), &mut corpus).unwrap();
        assert!(added2);
        assert_eq!(corpus.len(), 2);
    }

    #[test]
    fn unique_input_feedback_add_known() {
        let mut feedback = UniqueInputFeedback::new();
        let input1: Vec<u8> = vec![1, 2, 3];
        let observers_data = HashMap::new();
        let corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();

        feedback.add_known_input_hash(&input1);
        assert!(
            !feedback
                .is_interesting(&input1, &observers_data, &corpus)
                .unwrap()
        );
    }
}
