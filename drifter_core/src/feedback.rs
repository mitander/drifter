use crate::corpus::Corpus;
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
}

pub trait Feedback<I: Input, C: Corpus<I>>: Send + Sync {
    fn name(&self) -> &'static str;
    fn init(&mut self, corpus: &C) -> Result<(), FeedbackError>;
    fn is_interesting(
        &mut self,
        input: &I,
        observers_data: &HashMap<&'static str, Option<Vec<u8>>>,
        corpus: &C,
    ) -> Result<bool, FeedbackError>;
    fn process_execution(
        &mut self,
        executed_input_ref: &I,
        input_to_potentially_add: I,
        observers_data: &HashMap<&'static str, Option<Vec<u8>>>,
        corpus: &mut C,
    ) -> Result<bool, FeedbackError>;
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

impl<I: Input, C: Corpus<I>> Feedback<I, C> for UniqueInputFeedback {
    fn name(&self) -> &'static str {
        "UniqueInputFeedback"
    }

    fn init(&mut self, _: &C) -> Result<(), FeedbackError> {
        // If we want init to populate known_hashes from an existing corpus:
        // This assumes corpus can be iterated. Let's keep it simple for now
        // and assume seeding happens externally via `add_known_input_hash` or by
        // processing initial seeds through `process_execution`.
        // For example:
        // for i in 0..corpus.len() {
        //     if let Some((input, _)) = corpus.get(i) {
        //         self.add_known_input_hash(input);
        //     }
        // }
        Ok(())
    }

    fn is_interesting(
        &mut self,
        input: &I,
        _observers_data: &HashMap<&'static str, Option<Vec<u8>>>,
        _corpus: &C,
    ) -> Result<bool, FeedbackError> {
        let hash = md5::compute(input.as_bytes());
        Ok(!self.known_hashes.contains(&hash.0))
    }

    fn process_execution(
        &mut self,
        executed_input_ref: &I,
        input_to_potentially_add: I,
        _observers_data: &HashMap<&'static str, Option<Vec<u8>>>,
        corpus: &mut C,
    ) -> Result<bool, FeedbackError> {
        let hash = md5::compute(executed_input_ref.as_bytes());
        if self.known_hashes.insert(hash.0) {
            let metadata: Box<dyn Any + Send + Sync> =
                Box::new(format!("Unique hash: {:x}", md5::Digest(hash.0)));
            corpus.add(input_to_potentially_add, metadata)?;
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
        let added1 = feedback
            .process_execution(&input1, input1.clone(), &observers_data, &mut corpus)
            .unwrap();
        assert!(added1);
        assert_eq!(corpus.len(), 1);
        assert!(
            !feedback
                .is_interesting(&input1, &observers_data, &corpus)
                .unwrap()
        );
        let added1_again = feedback
            .process_execution(&input1, input1.clone(), &observers_data, &mut corpus)
            .unwrap();
        assert!(!added1_again);
        assert_eq!(corpus.len(), 1);

        let input2: Vec<u8> = vec![4, 5, 6];
        assert!(
            feedback
                .is_interesting(&input2, &observers_data, &corpus)
                .unwrap()
        );
        let added2 = feedback
            .process_execution(&input2, input2.clone(), &observers_data, &mut corpus)
            .unwrap();
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
