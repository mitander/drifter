use crate::corpus::Corpus;
use crate::input::Input;
use bincode::{Decode, Encode, config, decode_from_slice, encode_to_vec};
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

        let idx_to_mutate = if bytes.is_empty() {
            0
        } else {
            rng.random_range(0..bytes.len())
        };
        if !bytes.is_empty() {
            bytes[idx_to_mutate] = bytes[idx_to_mutate].wrapping_add(rng.random_range(1u8..=15u8));
        }
        Ok(I::from(bytes))
    }
}

pub struct JsonStructureAwareMutator<S, RngType>
where
    S: Serialize + DeserializeOwned + Clone + Default + Encode + Decode<()>,
    RngType: Rng + ?Sized,
{
    target_type_name: String,
    _marker_s: std::marker::PhantomData<S>,
    _marker_r: std::marker::PhantomData<RngType>,
    max_mutation_depth: usize,
    field_change_probability: f32,
    bincode_cfg: config::Configuration<config::LittleEndian, config::Fixint, config::NoLimit>,
}

impl<S, RngType> JsonStructureAwareMutator<S, RngType>
where
    S: Serialize + DeserializeOwned + Clone + Default + Encode + Decode<()>,
    RngType: Rng + ?Sized,
{
    pub fn new(
        target_type_name: String,
        max_mutation_depth: usize,
        field_change_probability: f32,
    ) -> Self {
        Self {
            target_type_name,
            _marker_s: std::marker::PhantomData,
            _marker_r: std::marker::PhantomData,
            max_mutation_depth,
            field_change_probability,
            bincode_cfg: config::standard()
                .with_little_endian()
                .with_fixed_int_encoding(),
        }
    }

    fn mutate_json_value(&self, value: &mut JsonValue, rng: &mut RngType, current_depth: usize) {
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
                        let delta = rng.random_range(-5..=5i64);
                        *n = JsonNumber::from(val_i64.saturating_add(delta));
                    } else if let Some(val_u64) = n.as_u64() {
                        let delta = rng.random_range(0..=5u64);
                        *n = JsonNumber::from(val_u64.saturating_add(delta));
                    } else if let Some(val_f64) = n.as_f64() {
                        let delta: f64 = rng.random_range(-1.0..1.0f64);
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

impl<S, RngType> Mutator<Vec<u8>, RngType> for JsonStructureAwareMutator<S, RngType>
where
    S: Serialize + DeserializeOwned + Clone + Default + Encode + Decode<()>,
    RngType: Rng + ?Sized,
{
    fn mutate(
        &mut self,
        input_opt: Option<&Vec<u8>>,
        rng: &mut RngType,
        _corpus_opt: Option<&dyn Corpus<Vec<u8>>>,
    ) -> Result<Vec<u8>, anyhow::Error> {
        let mut structured_obj: S = match input_opt {
            Some(bytes) => {
                if bytes.is_empty() {
                    S::default()
                } else {
                    match decode_from_slice(bytes, self.bincode_cfg) {
                        Ok((obj, _len)) => obj,
                        Err(_e) => {
                            return Ok(bytes.clone());
                        }
                    }
                }
            }
            None => S::default(),
        };
        let original_for_fallback = structured_obj.clone();
        let mut json_val: JsonValue = match serde_json::to_value(&structured_obj) {
            Ok(val) => val,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Failed to serialize {} to JSON: {}",
                    self.target_type_name,
                    e
                ));
            }
        };
        self.mutate_json_value(&mut json_val, rng, 0);
        structured_obj = match serde_json::from_value(json_val) {
            Ok(val) => val,
            Err(_e) => original_for_fallback,
        };
        match encode_to_vec(&structured_obj, self.bincode_cfg) {
            Ok(bytes) => Ok(bytes),
            Err(e) => Err(anyhow::anyhow!(
                "Failed to serialize mutated {} back to bytes: {}",
                self.target_type_name,
                e
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{Corpus, CorpusError};
    use bincode::{Decode, Encode};
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

    #[derive(Serialize, Deserialize, Encode, Decode, Debug, Clone, PartialEq, Eq, Default)]
    struct TestStruct {
        field_a: i32,
        field_b: bool,
        field_c: String,
        nested: Option<NestedStruct>,
    }
    #[derive(Serialize, Deserialize, Encode, Decode, Debug, Clone, PartialEq, Eq, Default)]
    struct NestedStruct {
        sub_field: u8,
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
    fn mutate_json_value_direct_test() {
        let mutator = JsonStructureAwareMutator::<TestStruct, ChaCha8Rng>::new(
            "TestStruct".to_string(),
            5,
            1.0,
        );
        let mut rng = ChaCha8Rng::from_seed([1u8; 32]);
        let mut val_bool = JsonValue::Bool(true);
        mutator.mutate_json_value(&mut val_bool, &mut rng, 0);
        assert_eq!(val_bool, JsonValue::Bool(false));
    }

    #[test]
    fn json_structure_aware_mutator_vec_u8_flow() {
        let mut mutator = JsonStructureAwareMutator::<TestStruct, ChaCha8Rng>::new(
            "TestStruct".to_string(),
            5,
            0.8,
        );
        let mut rng = ChaCha8Rng::from_seed([42u8; 32]);
        let initial_struct = TestStruct {
            field_a: 10,
            field_b: true,
            field_c: "hello".to_string(),
            nested: Some(NestedStruct { sub_field: 100 }),
        };
        let bincode_test_cfg = config::standard()
            .with_little_endian()
            .with_fixed_int_encoding();
        let initial_bytes = encode_to_vec(&initial_struct, bincode_test_cfg).unwrap();
        let dummy_corpus = DummyCorpus {};
        let mut changed_count = 0;
        for i in 0..200 {
            let mutated_bytes = mutator
                .mutate(Some(&initial_bytes), &mut rng, Some(&dummy_corpus))
                .unwrap();
            let (deserialized_mutated_struct, _): (TestStruct, usize) =
                match decode_from_slice(&mutated_bytes, bincode_test_cfg) {
                    Ok(res) => res,
                    Err(e) => panic!(
                        "Bincode deserialization of mutated_bytes failed (iter {}): {}",
                        i, e
                    ),
                };
            if deserialized_mutated_struct != initial_struct {
                changed_count += 1;
            }
        }
        assert!(
            changed_count > 0,
            "Mutator should have changed content at least once"
        );
    }

    #[test]
    fn json_structure_aware_mutator_handles_empty_or_bad_input_bytes() {
        let mut mutator = JsonStructureAwareMutator::<TestStruct, ChaCha8Rng>::new(
            "TestStruct".to_string(),
            5,
            0.8,
        );
        let mut rng = ChaCha8Rng::from_seed([43u8; 32]);
        let dummy_corpus = DummyCorpus {};
        let empty_bytes = Vec::new();
        let mutated_from_empty = mutator
            .mutate(Some(&empty_bytes), &mut rng, Some(&dummy_corpus))
            .unwrap();
        let (default_struct, _): (TestStruct, usize) =
            decode_from_slice(&mutated_from_empty, mutator.bincode_cfg).unwrap();
        let _ = default_struct;
        let garbage_bytes = vec![1, 2, 3, 4, 5, 255, 254, 253];
        let mutated_from_garbage = mutator
            .mutate(Some(&garbage_bytes), &mut rng, Some(&dummy_corpus))
            .unwrap();
        assert_eq!(mutated_from_garbage, garbage_bytes);
        let generated_bytes = mutator.mutate(None, &mut rng, Some(&dummy_corpus)).unwrap();
        let (generated_struct, _): (TestStruct, usize) =
            decode_from_slice(&generated_bytes, mutator.bincode_cfg)
                .expect("Generated bytes should be valid TestStruct");
        let _ = generated_struct;
    }
}
