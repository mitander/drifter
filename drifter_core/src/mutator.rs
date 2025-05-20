use crate::corpus::Corpus;
use crate::input::Input;
use rand::Rng;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Number as JsonNumber, Value as JsonValue};

pub trait Mutator<I: Input, R: Rng + ?Sized> {
    fn mutate(
        &mut self,
        input_opt: Option<&I>,
        rng: &mut R,
        corpus_opt: Option<&dyn Corpus<I>>,
    ) -> Result<I, anyhow::Error>;
}

pub struct FlipSingleByteMutator;

impl<I, R> Mutator<I, R> for FlipSingleByteMutator
where
    I: Input + From<Vec<u8>> + Clone,
    Vec<u8>: From<I>,
    R: Rng + ?Sized,
{
    fn mutate(
        &mut self,
        input_opt: Option<&I>,
        rng: &mut R,
        _corpus_opt: Option<&dyn Corpus<I>>,
    ) -> Result<I, anyhow::Error> {
        let concrete_input: I = match input_opt {
            Some(input) => (*input).clone(),
            None => I::from(vec![0u8; 1]),
        };
        let mut bytes = Vec::from(concrete_input);

        if bytes.is_empty() {
            bytes.push(0);
        }

        let idx_to_mutate = rng.random_range(0..bytes.len());
        bytes[idx_to_mutate] = bytes[idx_to_mutate].wrapping_add(rng.random_range(1u8..=15u8));

        Ok(I::from(bytes))
    }
}

pub struct JsonStructureAwareMutator<I, R>
where
    I: Input + Serialize + DeserializeOwned + From<Vec<u8>> + Clone,
    Vec<u8>: From<I>,
    R: Rng + ?Sized,
{
    _marker_i: std::marker::PhantomData<I>,
    _marker_r: std::marker::PhantomData<R>,
    max_mutation_depth: usize,
    field_change_probability: f32,
}

impl<I, R> JsonStructureAwareMutator<I, R>
where
    I: Input + Serialize + DeserializeOwned + From<Vec<u8>> + Clone,
    Vec<u8>: From<I>,
    R: Rng + ?Sized,
{
    pub fn new(max_mutation_depth: usize, field_change_probability: f32) -> Self {
        Self {
            _marker_i: std::marker::PhantomData,
            _marker_r: std::marker::PhantomData,
            max_mutation_depth,
            field_change_probability,
        }
    }

    fn mutate_json_value(&self, value: &mut JsonValue, rng: &mut R, current_depth: usize) {
        if current_depth >= self.max_mutation_depth {
            return;
        }
        match value {
            JsonValue::Object(map) => {
                for (_key, val) in map.iter_mut() {
                    if rng.random_bool(self.field_change_probability as f64) {
                        self.mutate_json_value(val, rng, current_depth + 1);
                    }
                }
            }
            JsonValue::Array(arr) => {
                for val in arr.iter_mut() {
                    if rng.random_bool(self.field_change_probability as f64) {
                        self.mutate_json_value(val, rng, current_depth + 1);
                    }
                }
            }
            JsonValue::String(s) => {
                if !s.is_empty() && rng.random_bool(0.5) {
                    let mut chars: Vec<char> = s.chars().collect();
                    if !chars.is_empty() {
                        let idx = rng.random_range(0..chars.len());
                        if chars[idx].is_ascii_alphabetic() && rng.random_bool(0.5) {
                            if chars[idx].is_ascii_lowercase() {
                                chars[idx] = chars[idx].to_ascii_uppercase();
                            } else {
                                chars[idx] = chars[idx].to_ascii_lowercase();
                            }
                        } else {
                            chars[idx] = rng.random_range(32u8..127u8) as char;
                        }
                        *s = chars.into_iter().collect();
                    }
                }
            }
            JsonValue::Number(n) => {
                if rng.random_bool(0.5) {
                    if let Some(val_i64) = n.as_i64() {
                        let delta = rng.random_range(-5..=5);
                        *n = JsonNumber::from(val_i64.saturating_add(delta));
                    } else if let Some(val_u64) = n.as_u64() {
                        let delta = rng.random_range(0..=5);
                        *n = JsonNumber::from(val_u64.saturating_add(delta));
                    } else if let Some(val_f64) = n.as_f64() {
                        let delta: f64 = rng.random_range(-1.0..1.0);
                        *n = JsonNumber::from_f64(val_f64 + delta)
                            .unwrap_or_else(|| JsonNumber::from(0));
                    }
                }
            }
            JsonValue::Bool(b) => {
                if rng.random_bool(0.5) {
                    *b = !*b;
                }
            }
            JsonValue::Null => {}
        }
    }
}

impl<I, R> Mutator<I, R> for JsonStructureAwareMutator<I, R>
where
    I: Input + Serialize + DeserializeOwned + From<Vec<u8>> + Clone,
    Vec<u8>: From<I>,
    R: Rng + ?Sized,
{
    fn mutate(
        &mut self,
        input_opt: Option<&I>,
        rng: &mut R,
        _corpus_opt: Option<&dyn Corpus<I>>,
    ) -> Result<I, anyhow::Error> {
        let concrete_input: I = match input_opt {
            Some(input) => (*input).clone(),
            None => {
                return Err(anyhow::anyhow!(
                    "JsonStructureAwareMutator requires an initial input to mutate."
                ));
            }
        };
        let mut json_val: JsonValue = serde_json::to_value(concrete_input)?;
        self.mutate_json_value(&mut json_val, rng, 0);
        match serde_json::from_value(json_val) {
            Ok(val) => Ok(val),
            Err(e) => {
                if let Some(input) = input_opt {
                    Ok((*input).clone())
                } else {
                    Err(anyhow::anyhow!(
                        "Mutation resulted in invalid JSON structure for type I: {}",
                        e
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{Corpus, CorpusError};
    use rand_chacha::ChaCha8Rng;
    use rand_core::{RngCore, SeedableRng};
    use serde::{Deserialize, Serialize};
    use std::any::Any;
    use std::path::PathBuf;

    struct DummyCorpus;
    impl<T: Input> Corpus<T> for DummyCorpus {
        fn add(
            &mut self,
            _input: T,
            _metadata: Box<dyn Any + Send + Sync>,
        ) -> Result<usize, CorpusError> {
            Ok(0)
        }
        fn get(&mut self, _id: usize) -> Option<(&T, &Box<dyn Any + Send + Sync>)> {
            None
        }
        fn random_select(
            &mut self,
            _rng: &mut dyn RngCore,
        ) -> Option<(usize, &T, &Box<dyn Any + Send + Sync>)> {
            None
        }
        fn len(&self) -> usize {
            0
        }
        fn load_initial_seeds(&mut self, _seed_paths: &[PathBuf]) -> Result<usize, CorpusError> {
            Ok(0)
        }
    }

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
    struct TestStruct {
        field_a: i32,
        field_b: bool,
        field_c: String,
        nested: Option<NestedStruct>,
    }
    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
    struct NestedStruct {
        sub_field: u8,
    }
    impl Input for TestStruct {
        fn as_bytes(&self) -> &[u8] {
            panic!("Not used in this test")
        }
        fn len(&self) -> usize {
            0
        }
        fn is_empty(&self) -> bool {
            true
        }
    }
    impl From<Vec<u8>> for TestStruct {
        fn from(_vec: Vec<u8>) -> Self {
            TestStruct::default()
        }
    }
    impl From<TestStruct> for Vec<u8> {
        fn from(_s: TestStruct) -> Vec<u8> {
            vec![]
        }
    }

    #[test]
    fn flip_single_byte_mutator_works_with_rng() {
        let mut mutator = FlipSingleByteMutator;
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let initial_input: Vec<u8> = vec![10, 20, 30];
        let dummy_corpus = DummyCorpus {};
        let mutated_input = mutator
            .mutate(Some(&initial_input), &mut rng, Some(&dummy_corpus))
            .unwrap();
        assert_ne!(initial_input, mutated_input);
    }

    #[test]
    fn json_structure_aware_mutator_works() {
        let mut mutator = JsonStructureAwareMutator::<TestStruct, ChaCha8Rng>::new(5, 0.5);
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let initial_input = TestStruct {
            ..Default::default()
        };
        let dummy_corpus = DummyCorpus {};
        let mut changed_count = 0;
        for _ in 0..100 {
            let mutated_input = mutator
                .mutate(Some(&initial_input), &mut rng, Some(&dummy_corpus))
                .unwrap();
            if mutated_input != initial_input {
                changed_count += 1;
            }
        }
        assert!(changed_count > 0);
    }

    #[test]
    fn mutate_json_value_direct_test() {
        let mutator = JsonStructureAwareMutator::<TestStruct, ChaCha8Rng>::new(5, 1.0);
        let mut rng = ChaCha8Rng::from_seed([1u8; 32]);
        let mut val_bool = JsonValue::Bool(true);
        mutator.mutate_json_value(&mut val_bool, &mut rng, 0);
        assert_eq!(val_bool, JsonValue::Bool(false));
    }
}
