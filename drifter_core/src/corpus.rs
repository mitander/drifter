use crate::input::Input;
use rand_core::RngCore;
use std::any::Any;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CorpusError {
    #[error("Input ID {0} not found in corpus")]
    NotFound(usize),
    #[error("Corpus is empty, cannot select an input")]
    Empty,
}

pub trait Corpus<I: Input>: Send + Sync {
    fn add(&mut self, input: I, metadata: Box<dyn Any + Send + Sync>)
    -> Result<usize, CorpusError>;
    fn get(&self, id: usize) -> Option<(&I, &Box<dyn Any + Send + Sync>)>;
    fn random_select(
        &self,
        rng: &mut dyn RngCore,
    ) -> Option<(usize, &I, &Box<dyn Any + Send + Sync>)>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct InMemoryCorpus<I: Input> {
    entries: Vec<(I, Box<dyn Any + Send + Sync>)>,
}

impl<I: Input> InMemoryCorpus<I> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<I: Input> Default for InMemoryCorpus<I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: Input> Corpus<I> for InMemoryCorpus<I> {
    fn add(
        &mut self,
        input: I,
        metadata: Box<dyn Any + Send + Sync>,
    ) -> Result<usize, CorpusError> {
        let id = self.entries.len();
        self.entries.push((input, metadata));
        Ok(id)
    }

    fn get(&self, id: usize) -> Option<(&I, &Box<dyn Any + Send + Sync>)> {
        self.entries.get(id).map(|(inp, meta)| (inp, meta))
    }

    fn random_select(
        &self,
        rng: &mut dyn RngCore,
    ) -> Option<(usize, &I, &Box<dyn Any + Send + Sync>)> {
        if self.is_empty() {
            return None;
        }
        let idx = rng.next_u32() as usize % self.entries.len();
        self.entries.get(idx).map(|(inp, meta)| (idx, inp, meta))
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha8Rng;
    use rand_core::SeedableRng;

    #[derive(Debug, PartialEq, Eq)]
    struct TestMetadata {
        info: String,
    }

    #[test]
    fn in_memory_corpus_add_and_get() {
        let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
        let input1 = vec![1, 2, 3];
        let meta1 = Box::new(TestMetadata {
            info: "first".to_string(),
        });
        let id1 = corpus.add(input1.clone(), meta1).unwrap();
        assert_eq!(id1, 0);

        let input2 = vec![4, 5, 6];
        let meta2 = Box::new(TestMetadata {
            info: "second".to_string(),
        });
        let id2 = corpus.add(input2.clone(), meta2).unwrap();
        assert_eq!(id2, 1);

        assert_eq!(corpus.len(), 2);

        let (ret_input1, ret_meta1) = corpus.get(id1).unwrap();
        assert_eq!(*ret_input1, input1);
        assert_eq!(
            ret_meta1.downcast_ref::<TestMetadata>().unwrap().info,
            "first"
        );

        let (ret_input2, ret_meta2) = corpus.get(id2).unwrap();
        assert_eq!(*ret_input2, input2);
        assert_eq!(
            ret_meta2.downcast_ref::<TestMetadata>().unwrap().info,
            "second"
        );
    }

    #[test]
    fn in_memory_corpus_random_select() {
        let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
        assert!(
            corpus
                .random_select(&mut ChaCha8Rng::from_seed([0; 32]))
                .is_none()
        );

        let input1 = vec![1];
        corpus
            .add(
                input1.clone(),
                Box::new(TestMetadata {
                    info: "".to_string(),
                }),
            )
            .unwrap();
        let input2 = vec![2];
        corpus
            .add(
                input2.clone(),
                Box::new(TestMetadata {
                    info: "".to_string(),
                }),
            )
            .unwrap();

        let mut rng = ChaCha8Rng::from_seed([0; 32]);
        let selection_count = 100;
        let mut selected_ids = std::collections::HashSet::new();
        for _ in 0..selection_count {
            if let Some((id, _, _)) = corpus.random_select(&mut rng) {
                selected_ids.insert(id);
            }
        }
        assert!(selected_ids.contains(&0));
        assert!(selected_ids.contains(&1));
    }

    #[test]
    fn in_memory_corpus_is_empty() {
        let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
        assert!(corpus.is_empty());
        corpus
            .add(
                vec![1],
                Box::new(TestMetadata {
                    info: "".to_string(),
                }),
            )
            .unwrap();
        assert!(!corpus.is_empty());
    }
}
