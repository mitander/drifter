use crate::corpus::Corpus;
use crate::input::Input;
use rand_core::RngCore;

pub trait Mutator<I: Input> {
    fn mutate(
        &mut self,
        input_opt: Option<&I>,
        rng: &mut dyn RngCore,
        corpus_opt: Option<&dyn Corpus<I>>,
    ) -> Result<I, anyhow::Error>;
}

pub struct FlipSingleByteMutator;

impl<I> Mutator<I> for FlipSingleByteMutator
where
    I: Input + From<Vec<u8>>,
    Vec<u8>: From<I>,
{
    fn mutate(
        &mut self,
        input_opt: Option<&I>,
        rng: &mut dyn RngCore,
        _corpus_opt: Option<&dyn Corpus<I>>,
    ) -> Result<I, anyhow::Error> {
        let mut bytes = match input_opt {
            Some(inp) => Vec::from(inp.clone()),
            None => vec![0u8; 1],
        };

        if bytes.is_empty() {
            bytes.push(0);
        }

        let idx_to_mutate = rng.next_u32() as usize % bytes.len();
        bytes[idx_to_mutate] = bytes[idx_to_mutate].wrapping_add(1 + (rng.next_u32() % 15) as u8);

        Ok(I::from(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{Corpus, CorpusError};
    use crate::input::Input;
    use rand_chacha::ChaCha8Rng;
    use rand_core::{RngCore, SeedableRng};
    use std::any::Any;

    struct DummyCorpus;
    impl<I: Input> Corpus<I> for DummyCorpus {
        fn add(
            &mut self,
            _input: I,
            _metadata: Box<dyn Any + Send + Sync>,
        ) -> Result<usize, CorpusError> {
            Ok(0)
        }
        fn get(&self, _id: usize) -> Option<(&I, &Box<dyn Any + Send + Sync>)> {
            None
        }
        fn random_select(
            &self,
            _rng: &mut dyn RngCore,
        ) -> Option<(usize, &I, &Box<dyn Any + Send + Sync>)> {
            None
        }
        fn len(&self) -> usize {
            0
        }
    }

    #[test]
    fn flip_single_byte_mutator_works() {
        let mut mutator = FlipSingleByteMutator;
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let initial_input: Vec<u8> = vec![10, 20, 30];

        let dummy_corpus_instance = DummyCorpus;
        let dummy_corpus_opt: Option<&dyn Corpus<Vec<u8>>> = Some(&dummy_corpus_instance);

        let mutated_input = mutator
            .mutate(Some(&initial_input), &mut rng, dummy_corpus_opt)
            .unwrap();

        assert_ne!(initial_input, mutated_input);
        assert_eq!(initial_input.len(), mutated_input.len());

        let generated_input = mutator.mutate(None, &mut rng, dummy_corpus_opt).unwrap();
        assert!(!generated_input.is_empty());
    }
}
