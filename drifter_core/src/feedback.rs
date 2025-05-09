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
    fn report_interesting(
        &mut self,
        input: I,
        observers_data: &HashMap<&'static str, Option<Vec<u8>>>,
        corpus: &mut C,
    ) -> Result<(), FeedbackError>;
}

#[derive(Default)]
pub struct UniqueInputFeedback {
    pub known_hashes: HashSet<[u8; 16]>,
}

impl UniqueInputFeedback {
    pub fn new() -> Self {
        Self {
            known_hashes: HashSet::new(),
        }
    }
}

impl<I: Input, C: Corpus<I>> Feedback<I, C> for UniqueInputFeedback {
    fn name(&self) -> &'static str {
        "UniqueInputFeedback"
    }

    fn init(&mut self, _corpus: &C) -> Result<(), FeedbackError> {
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

    fn report_interesting(
        &mut self,
        input: I,
        _observers_data: &HashMap<&'static str, Option<Vec<u8>>>,
        corpus: &mut C,
    ) -> Result<(), FeedbackError> {
        let hash = md5::compute(input.as_bytes());
        if self.known_hashes.insert(hash.0) {
            let metadata: Box<dyn Any + Send + Sync> = Box::new(hash.0);
            corpus.add(input, metadata)?;
        }
        Ok(())
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
        feedback
            .report_interesting(input1.clone(), &observers_data, &mut corpus)
            .unwrap();
        assert_eq!(corpus.len(), 1);
        assert!(
            !feedback
                .is_interesting(&input1, &observers_data, &corpus)
                .unwrap()
        );

        let input2: Vec<u8> = vec![4, 5, 6];
        assert!(
            feedback
                .is_interesting(&input2, &observers_data, &corpus)
                .unwrap()
        );
        feedback
            .report_interesting(input2.clone(), &observers_data, &mut corpus)
            .unwrap();
        assert_eq!(corpus.len(), 2);

        let initial_corpus_len = corpus.len();
        feedback
            .report_interesting(input1.clone(), &observers_data, &mut corpus)
            .unwrap();
        assert_eq!(corpus.len(), initial_corpus_len);
    }
}
