use crate::corpus::Corpus;
use crate::input::Input;
use bincode::{
    Decode, Encode,
    config::{Configuration, Fixint, LittleEndian, NoLimit},
    decode_from_slice, encode_to_vec,
};
use rand::Rng;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Number as JsonNumber, Value as JsonValue};
use std::marker::PhantomData;

/// Defines a probability for a field to change during JSON mutation, if not overridden.
const DEFAULT_JSON_FIELD_CHANGE_PROBABILITY: f32 = 0.5;
/// Defines a probability for a specific mutation type (e.g., string char change, number delta) to occur.
const DEFAULT_JSON_VALUE_MUTATION_PROBABILITY: f64 = 0.5;
/// Defines the default maximum recursion depth for `JsonStructureAwareMutator`.
const DEFAULT_JSON_MAX_MUTATION_DEPTH: usize = 10;

/// A `Mutator` is responsible for transforming an `Input` into a new, potentially modified `Input`.
///
/// Mutators are the core engine for generating new test cases from existing ones in a fuzzing loop.
/// They can implement various strategies, from simple byte flips to complex, structure-aware
/// transformations.
///
/// # Type Parameters
/// * `I`: The type of `Input` this mutator operates on.
/// * `R`: The type of random number generator used for mutation decisions.
pub trait Mutator<I: Input, R: Rng + ?Sized> {
    /// Applies a mutation strategy to an optional input to produce a new input.
    ///
    /// # Arguments
    /// * `input_opt`: An `Option<&I>` representing the input to mutate.
    ///   - `Some(input)`: The mutator will base its transformation on this input.
    ///   - `None`: The mutator might generate a completely new input from scratch or
    ///     use a default starting point.
    /// * `rng`: A mutable reference to a random number generator.
    /// * `corpus_opt`: An `Option<&dyn Corpus<I>>` providing access to the current corpus.
    ///   Some mutators might use the corpus to draw data for splicing or to inform
    ///   their mutations (e.g., dictionary-based mutators). This is optional.
    ///
    /// # Returns
    /// `Result<I, anyhow::Error>`:
    ///   - `Ok(new_input)`: The newly generated or mutated input.
    ///   - `Err(error)`: If any error occurred during the mutation process.
    fn mutate(
        &mut self,
        input_opt: Option<&I>,
        rng: &mut R,
        corpus_opt: Option<&dyn Corpus<I>>,
    ) -> Result<I, anyhow::Error>;
}

/// A simple `Mutator` that randomly selects a single byte in the input and
/// flips a few of its bits by adding a small random value.
///
/// If the input is empty, it first creates a single zero byte.
/// If no input is provided (`None`), it starts with a single zero byte.
#[derive(Debug, Default, Clone, Copy)]
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
        let base_input: I = match input_opt {
            Some(input_ref) => input_ref.clone(),
            None => I::from(vec![0u8; 1]),
        };

        let mut input_bytes = Vec::from(base_input);

        if input_bytes.is_empty() {
            // Ensure there's at least one byte to mutate
            input_bytes.push(0);
        }

        // Add a small random value (1-15) to the selected byte, with wrapping.
        // `random_range` for inclusive range is `min..=max`.
        let random_add_value = rng.random_range(1u8..=15u8);
        // `random_range` is exclusive for the upper bound, so `0..len` is correct.
        let byte_index_to_mutate = rng.random_range(0..input_bytes.len());

        input_bytes[byte_index_to_mutate] =
            input_bytes[byte_index_to_mutate].wrapping_add(random_add_value);

        Ok(I::from(input_bytes))
    }
}

/// A structure-aware `Mutator` that deserializes a binary `Input` (expected to be `Vec<u8>`
/// representing a bincode-serialized structure `S`) into `S`, then converts `S` to a
/// `serde_json::Value`, mutates the JSON representation, converts it back to `S`,
/// and finally serializes `S` back to `Vec<u8>`.
///
/// This allows for mutations that respect the fields and types of the structured data `S`,
/// rather than just random byte changes.
///
/// # Type Parameters
/// * `S`: The structured type that inputs represent. Must implement `Serialize`,
///   `DeserializeOwned`, `Clone`, `Default`, `Encode`, and `Decode<()>`.
/// * `RngType`: The specific type of random number generator to be used.
pub struct JsonStructureAwareMutator<S, RngType>
where
    S: Serialize + DeserializeOwned + Clone + Default + Encode + Decode<()>,
    RngType: Rng + ?Sized,
{
    /// A descriptive name for the target type `S`, used in error messages.
    target_type_name: String,
    /// Maximum recursion depth when mutating nested JSON structures (objects/arrays).
    max_mutation_depth: usize,
    /// Probability (0.0 to 1.0) that a field within a JSON object or an element
    /// within a JSON array will be selected for further mutation.
    field_recurse_probability: f64,
    /// Bincode configuration for serializing/deserializing the structured type `S`.
    bincode_config: Configuration<LittleEndian, Fixint, NoLimit>,
    _marker_s: PhantomData<S>,
    _marker_rng: PhantomData<RngType>,
}

impl<S, RngType> JsonStructureAwareMutator<S, RngType>
where
    S: Serialize + DeserializeOwned + Clone + Default + Encode + Decode<()>,
    RngType: Rng + ?Sized,
{
    /// Creates a new `JsonStructureAwareMutator`.
    ///
    /// # Arguments
    /// * `target_type_name`: A string identifying the type `S` (for logging/errors).
    /// * `max_mutation_depth`: Max recursion depth for mutating nested JSON.
    /// * `field_recurse_probability`: Probability (0.0-1.0) to recurse into a JSON field/element.
    pub fn new(
        target_type_name: String,
        max_mutation_depth: usize,
        field_recurse_probability: f64,
    ) -> Self {
        Self {
            target_type_name,
            max_mutation_depth: if max_mutation_depth == 0 {
                DEFAULT_JSON_MAX_MUTATION_DEPTH
            } else {
                max_mutation_depth
            },
            field_recurse_probability: if field_recurse_probability <= 0.0
                || field_recurse_probability > 1.0
            {
                DEFAULT_JSON_FIELD_CHANGE_PROBABILITY as f64
            } else {
                field_recurse_probability
            },
            bincode_config: bincode::config::standard()
                .with_little_endian()
                .with_fixed_int_encoding(),
            _marker_s: PhantomData,
            _marker_rng: PhantomData,
        }
    }

    /// Recursively mutates a `serde_json::Value`.
    ///
    /// This is the core logic for how JSON values are altered. It handles different
    /// JSON types (Object, Array, String, Number, Bool, Null) with specific mutation
    /// strategies for each.
    ///
    /// # Arguments
    /// * `value`: A mutable reference to the `JsonValue` to be mutated.
    /// * `rng`: The random number generator.
    /// * `current_depth`: The current recursion depth, to prevent infinite loops with `max_mutation_depth`.
    fn mutate_json_value(&self, value: &mut JsonValue, rng: &mut RngType, current_depth: usize) {
        if current_depth >= self.max_mutation_depth {
            return;
        }

        match value {
            JsonValue::Object(map) => {
                for (_key, val) in map.iter_mut() {
                    if rng.random_bool(self.field_recurse_probability) {
                        self.mutate_json_value(val, rng, current_depth + 1);
                    }
                }
            }
            JsonValue::Array(arr) => {
                for val in arr.iter_mut() {
                    if rng.random_bool(self.field_recurse_probability) {
                        self.mutate_json_value(val, rng, current_depth + 1);
                    }
                }
            }
            JsonValue::String(s) => {
                if !s.is_empty() && rng.random_bool(DEFAULT_JSON_VALUE_MUTATION_PROBABILITY) {
                    let mut chars: Vec<char> = s.chars().collect();
                    let char_idx_to_mutate = rng.random_range(0..chars.len());

                    // Mutation: change case or pick a random printable ASCII char
                    if chars[char_idx_to_mutate].is_ascii_alphabetic() && rng.random_bool(0.5) {
                        if chars[char_idx_to_mutate].is_ascii_lowercase() {
                            chars[char_idx_to_mutate] =
                                chars[char_idx_to_mutate].to_ascii_uppercase();
                        } else {
                            chars[char_idx_to_mutate] =
                                chars[char_idx_to_mutate].to_ascii_lowercase();
                        }
                    } else {
                        // Replace with a random printable ASCII character (32-127)
                        chars[char_idx_to_mutate] = rng.random_range(32u8..127u8) as char;
                    }
                    *s = chars.into_iter().collect();
                }
            }
            JsonValue::Number(n) => {
                if rng.random_bool(DEFAULT_JSON_VALUE_MUTATION_PROBABILITY) {
                    if let Some(val_i64) = n.as_i64() {
                        let delta = rng.random_range(-5i64..=5i64);
                        *n = JsonNumber::from(val_i64.saturating_add(delta));
                    } else if let Some(val_u64) = n.as_u64() {
                        let delta = rng.random_range(0u64..=5u64);
                        *n = JsonNumber::from(val_u64.saturating_add(delta));
                    } else if let Some(val_f64) = n.as_f64() {
                        let delta: f64 = rng.random_range(-1.0..1.0);
                        let new_val = val_f64 + delta;
                        if new_val.is_finite() {
                            *n = JsonNumber::from_f64(new_val)
                                .unwrap_or_else(|| JsonNumber::from(0));
                        } else {
                            *n = JsonNumber::from(0);
                        }
                    }
                }
            }
            JsonValue::Bool(b) => {
                if rng.random_bool(DEFAULT_JSON_VALUE_MUTATION_PROBABILITY) {
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
    /// Mutates a `Vec<u8>` input, assuming it's a bincode-serialized representation of structure `S`.
    ///
    /// The process involves:
    /// 1. Deserialize `Vec<u8>` to `S` (using bincode).
    ///    - If input is `None` or empty, `S::default()` is used.
    ///    - If deserialization fails, the original `Vec<u8>` might be returned (or an error).
    /// 2. Serialize `S` to `serde_json::Value`.
    /// 3. Mutate the `JsonValue` using `mutate_json_value`.
    /// 4. Deserialize the mutated `JsonValue` back to `S`.
    ///    - If this deserialization fails, the pre-JSON-mutation `S` is used as a fallback.
    /// 5. Serialize `S` back to `Vec<u8>` (using bincode).
    ///
    /// This mutator does not currently use the `corpus_opt`.
    fn mutate(
        &mut self,
        input_bytes_opt: Option<&Vec<u8>>,
        rng: &mut RngType,
        _corpus_opt: Option<&dyn Corpus<Vec<u8>>>,
    ) -> Result<Vec<u8>, anyhow::Error> {
        // Step 1: Obtain the initial structured object `S`
        let mut structured_s_object: S = match input_bytes_opt {
            Some(bytes) if !bytes.is_empty() => {
                // Attempt to decode from bincode; if fails, fallback to S::default()
                // rather than returning original bytes, to ensure mutation attempt on a valid S.
                match decode_from_slice(bytes, self.bincode_config) {
                    Ok((decoded_object, _length)) => decoded_object,
                    Err(decode_err) => {
                        eprintln!(
                            "Warning: Bincode deserialization of input failed for {}: {}. Mutating S::default().",
                            self.target_type_name, decode_err
                        );
                        S::default()
                    }
                }
            }
            _ => S::default(),
        };

        // Clone for fallback in case JSON processing corrupts the structure irrecoverably
        let original_s_for_fallback = structured_s_object.clone();

        // Step 2: Serialize S to JsonValue
        let mut json_representation: JsonValue = match serde_json::to_value(&structured_s_object) {
            Ok(value) => value,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Failed to serialize {} to JSON: {}. Input might be malformed or S's Serialize impl is problematic.",
                    self.target_type_name,
                    e
                ));
            }
        };

        // Step 3: Mutate the JsonValue
        self.mutate_json_value(&mut json_representation, rng, 0);

        // Step 4: Deserialize mutated JsonValue back to S
        structured_s_object = match serde_json::from_value(json_representation) {
            Ok(deserialized_s) => deserialized_s,
            Err(json_decode_err) => {
                eprintln!(
                    "Warning: Failed to deserialize mutated JSON back to {}: {}. Reverting to pre-JSON-mutation state.",
                    self.target_type_name, json_decode_err
                );
                // revert to the pre-mutation version of S.
                original_s_for_fallback
            }
        };

        // Step 5: Serialize S back to Vec<u8> using bincode
        match encode_to_vec(&structured_s_object, self.bincode_config) {
            Ok(output_bytes) => Ok(output_bytes),
            Err(e) => Err(anyhow::anyhow!(
                "Failed to serialize mutated {} back to bytes using bincode: {}",
                self.target_type_name,
                e
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::test_utils::DummyTestCorpus;
    use bincode::{Decode, Encode, config as bincode_config_rs};
    use rand_chacha::ChaCha8Rng;
    use rand_core::SeedableRng;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Encode, Decode, Debug, Clone, PartialEq, Eq, Default)]
    struct TestMutationStruct {
        field_a_int: i32,
        field_b_bool: bool,
        field_c_string: String,
        nested_struct: Option<NestedMutationStruct>,
    }

    #[derive(Serialize, Deserialize, Encode, Decode, Debug, Clone, PartialEq, Eq, Default)]
    struct NestedMutationStruct {
        sub_field_u8: u8,
    }

    #[test]
    fn flip_single_byte_mutator_mutates_input() {
        let mut mutator = FlipSingleByteMutator;
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let initial_input_bytes: Vec<u8> = vec![10, 20, 30];
        let dummy_corpus = DummyTestCorpus;

        let mutated_input_result =
            mutator.mutate(Some(&initial_input_bytes), &mut rng, Some(&dummy_corpus));

        assert!(mutated_input_result.is_ok(), "Mutation should succeed");
        let mutated_input_bytes = mutated_input_result.unwrap();
        assert_ne!(
            initial_input_bytes, mutated_input_bytes,
            "Mutated input should differ from initial input"
        );
        assert_eq!(
            mutated_input_bytes.len(),
            initial_input_bytes.len(),
            "Length should remain the same for this mutator"
        );
    }

    #[test]
    fn flip_single_byte_mutator_handles_empty_input() {
        let mut mutator = FlipSingleByteMutator;
        let mut rng = ChaCha8Rng::from_seed([1u8; 32]);
        let empty_input: Vec<u8> = vec![];
        let dummy_corpus = DummyTestCorpus;

        let mutated_from_empty = mutator
            .mutate(Some(&empty_input), &mut rng, Some(&dummy_corpus))
            .unwrap();
        assert_eq!(
            mutated_from_empty.len(),
            1,
            "Empty input should result in a 1-byte mutated output"
        );
    }

    #[test]
    fn flip_single_byte_mutator_handles_none_input() {
        let mut mutator = FlipSingleByteMutator;
        let mut rng = ChaCha8Rng::from_seed([2u8; 32]);
        let dummy_corpus = DummyTestCorpus;

        let mutated_from_none: Vec<u8> =
            mutator.mutate(None, &mut rng, Some(&dummy_corpus)).unwrap();
        assert_eq!(
            mutated_from_none.len(),
            1,
            "None input should result in a 1-byte mutated output"
        );
    }

    #[test]
    fn json_structure_aware_mutator_e2e_flow_with_vec_u8() {
        let mut mutator = JsonStructureAwareMutator::<TestMutationStruct, ChaCha8Rng>::new(
            "TestMutationStruct".to_string(),
            DEFAULT_JSON_MAX_MUTATION_DEPTH,
            0.8, // Reasonably high probability
        );
        let mut rng = ChaCha8Rng::from_seed([42u8; 32]);

        let initial_struct = TestMutationStruct {
            field_a_int: 10,
            field_b_bool: true,
            field_c_string: "hello_world_test".to_string(),
            nested_struct: Some(NestedMutationStruct { sub_field_u8: 100 }),
        };

        let bincode_test_cfg = bincode_config_rs::standard()
            .with_little_endian()
            .with_fixed_int_encoding();
        let initial_bytes = encode_to_vec(&initial_struct, bincode_test_cfg)
            .expect("Initial bincode encoding failed");

        let dummy_corpus = DummyTestCorpus;
        let mut changed_count = 0;
        let iterations = 200;

        for i in 0..iterations {
            let mutated_bytes_result =
                mutator.mutate(Some(&initial_bytes), &mut rng, Some(&dummy_corpus));
            assert!(
                mutated_bytes_result.is_ok(),
                "Mutation failed at iteration {}",
                i
            );
            let mutated_bytes = mutated_bytes_result.unwrap();

            // It's possible, though unlikely with high probability, that a mutation results in the exact same byte output.
            // The core check is if the deserialized structure changes.
            if mutated_bytes == initial_bytes && i < iterations / 2 {
                // Allow some initial mutations to be no-ops by chance
                // Continue if bytes are same, but we expect changes over many iterations
            }

            let (deserialized_mutated_struct, _len): (TestMutationStruct, usize) =
                match decode_from_slice(&mutated_bytes, bincode_test_cfg) {
                    Ok(res) => res,
                    Err(e) => panic!(
                        "Bincode deserialization of mutated_bytes failed (iter {}): {}. Bytes: {:?}",
                        i, e, mutated_bytes
                    ),
                };

            if deserialized_mutated_struct != initial_struct {
                changed_count += 1;
            }
        }
        assert!(
            changed_count > 0,
            "Mutator should have changed the deserialized structure content at least once over {} iterations. Changes: {}",
            iterations,
            changed_count
        );
    }

    #[test]
    fn json_structure_aware_mutator_handles_empty_and_none_input_bytes() {
        let mut mutator = JsonStructureAwareMutator::<TestMutationStruct, ChaCha8Rng>::new(
            "TestMutationStruct".to_string(),
            DEFAULT_JSON_MAX_MUTATION_DEPTH,
            0.8,
        );
        let mut rng = ChaCha8Rng::from_seed([43u8; 32]);
        let dummy_corpus = DummyTestCorpus;

        let empty_bytes = Vec::new();
        let mutated_from_empty_result =
            mutator.mutate(Some(&empty_bytes), &mut rng, Some(&dummy_corpus));
        assert!(
            mutated_from_empty_result.is_ok(),
            "Mutation from empty bytes should succeed"
        );
        let mutated_from_empty = mutated_from_empty_result.unwrap();

        let (default_struct_from_empty, _): (TestMutationStruct, usize) =
            decode_from_slice(&mutated_from_empty, mutator.bincode_config).expect(
                "Should deserialize from empty input mutation (likely mutated S::default())",
            );
        let _ = default_struct_from_empty;

        let mutated_from_none_result = mutator.mutate(None, &mut rng, Some(&dummy_corpus));
        assert!(
            mutated_from_none_result.is_ok(),
            "Mutation from None input should succeed"
        );
        let mutated_from_none = mutated_from_none_result.unwrap();

        let (default_struct_from_none, _): (TestMutationStruct, usize) =
            decode_from_slice(&mutated_from_none, mutator.bincode_config)
                .expect("Should deserialize from None input mutation (mutated S::default())");
        let _ = default_struct_from_none;
        let default_s_bytes =
            encode_to_vec(TestMutationStruct::default(), mutator.bincode_config).unwrap();
        if default_s_bytes.is_empty() {
            // It's statistically likely to be different after many internal mutation attempts,
            // but not guaranteed for a single run. The goal is valid output.
        }

        let garbage_bytes = vec![1, 2, 3, 4, 5, 255, 254, 253, 123, 222, 100, 50];
        let mutated_from_garbage_result =
            mutator.mutate(Some(&garbage_bytes), &mut rng, Some(&dummy_corpus));
        assert!(
            mutated_from_garbage_result.is_ok(),
            "Mutation from garbage bytes should succeed (by mutating S::default())"
        );
        let mutated_from_garbage = mutated_from_garbage_result.unwrap();

        let (_struct_from_garbage, _): (TestMutationStruct, usize) =
            decode_from_slice(&mutated_from_garbage, mutator.bincode_config)
            .expect("Output from garbage input should still be deserializable to TestMutationStruct (as S::default() was mutated)");
    }

    #[test]
    fn json_mutator_mutate_json_value_directly_alters_values() {
        let mutator = JsonStructureAwareMutator::<TestMutationStruct, ChaCha8Rng>::new(
            "TestMutationStruct".to_string(),
            DEFAULT_JSON_MAX_MUTATION_DEPTH,
            1.0,
        );
        let mut rng = ChaCha8Rng::from_seed([1u8; 32]);

        // Test Bool
        let mut val_bool = JsonValue::Bool(true);
        mutator.mutate_json_value(&mut val_bool, &mut rng, 0); // rng state advances
        assert_eq!(val_bool, JsonValue::Bool(false), "Boolean should flip");

        // Test String
        let mut val_string = JsonValue::String("TestString".to_string());
        let original_string_clone = val_string.as_str().unwrap().to_string();
        let mut string_changed = false;

        // Try mutating the string
        for _ in 0..20 {
            // Try up to 20 times to hit the 50% chance
            mutator.mutate_json_value(&mut val_string, &mut rng, 0); // rng state advances each time
            if val_string.as_str().unwrap() != original_string_clone {
                string_changed = true;
                break;
            }
            // No mutation, reset to original for next attempt
            val_string = JsonValue::String(original_string_clone.clone());
        }
        assert!(
            string_changed,
            "String should change within several attempts with fixed seed and 0.5 internal probability"
        );

        // Test Number (i64)
        let mut val_number_i64 = JsonValue::Number(JsonNumber::from(100i64));
        let original_i64_val = 100i64;
        let mut number_changed = false;
        for _ in 0..20 {
            mutator.mutate_json_value(&mut val_number_i64, &mut rng, 0);
            if val_number_i64.as_i64().unwrap_or(original_i64_val) != original_i64_val {
                number_changed = true;
                break;
            }
            val_number_i64 = JsonValue::Number(JsonNumber::from(original_i64_val));
        }
        assert!(
            number_changed,
            "i64 Number should change within several attempts"
        );
    }
}
