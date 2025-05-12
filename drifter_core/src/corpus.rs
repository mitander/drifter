use crate::input::Input;
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CorpusError {
    #[error("Input ID {0} not found in corpus")]
    NotFound(usize),
    #[error("Corpus is empty, cannot select an input")]
    Empty,
    #[error("Corpus I/O error: {0}")] // New variant
    IoError(String),
    // TODO:
    // #[error("Serialization error: {0}")]
    // SerializationError(String),
    // #[error("Deserialization error: {0}")]
    // DeserializationError(String),
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OnDiskCorpusEntryMetadata {
    pub source_description: String,
    // TODO: Add other simple, serializable metadata fields here
    // pub discovery_time: u64, // epoch seconds
    // pub executions: u64,
}

pub struct OnDiskCorpus<I: Input + Serialize + for<'de> Deserialize<'de>> {
    corpus_dir: PathBuf,
    entries_index: Vec<(String, OnDiskCorpusEntryMetadata)>,
    current_id: usize,
    _input_marker: PhantomData<I>,
    // TODO: For quick duplicate check, if desired, though file existence check on hash can also work
    // known_input_hashes: HashSet<[u8;16]>,
}

impl<I: Input + Serialize + for<'de> Deserialize<'de>> OnDiskCorpus<I> {
    pub fn new(corpus_dir: PathBuf) -> Result<Self, CorpusError> {
        if !corpus_dir.exists() {
            fs::create_dir_all(&corpus_dir).map_err(|e| {
                CorpusError::IoError(format!(
                    "Failed to create corpus directory {corpus_dir:?}: {e}",
                ))
            })?;
        } else if !corpus_dir.is_dir() {
            return Err(CorpusError::IoError(format!(
                // Add IoError to CorpusError enum
                "Corpus path {corpus_dir:?} is not a directory",
            )));
        }
        // TODO: Implement loading existing corpus index from disk if present
        Ok(Self {
            corpus_dir,
            entries_index: Vec::new(),
            current_id: 0,
            _input_marker: PhantomData,
        })
    }

    fn get_file_path(&self, filename_stem: &str) -> PathBuf {
        self.corpus_dir.join(filename_stem).with_extension("input") // e.g., id_000000.input
    }

    // TODO: Implement saving/loading the entries_index to/from a manifest file
    // fn save_index(&self) -> Result<(), CorpusError> { ... }
    // fn load_index(&mut self) -> Result<(), CorpusError> { ... }
}

impl<I: Input + Serialize + for<'de> Deserialize<'de>> Corpus<I> for OnDiskCorpus<I> {
    fn add(
        &mut self,
        input: I,
        metadata: Box<dyn Any + Send + Sync>,
    ) -> Result<usize, CorpusError> {
        // For OnDiskCorpus, we need to decide how to handle the generic Box<dyn Any> metadata.
        // Option 1: Try to downcast to our specific OnDiskCorpusEntryMetadata
        // Option 2: For V1, ignore passed metadata and create a default one or use a simple string.
        // Option 3: Require metadata itself to be serializable (adds complexity to the trait user).

        // Let's go with Option 2 for simplicity in this MVP, but acknowledge the limitation.
        let on_disk_meta = if let Some(s_meta) = metadata.downcast_ref::<String>() {
            OnDiskCorpusEntryMetadata {
                source_description: s_meta.clone(),
            }
        } else if let Some(od_meta) = metadata.downcast_ref::<OnDiskCorpusEntryMetadata>() {
            od_meta.clone()
        } else {
            OnDiskCorpusEntryMetadata {
                source_description: format!("entry_{}", self.current_id),
            }
        };

        let filename_stem = format!("id_{:06}", self.current_id);
        let file_path = self.get_file_path(&filename_stem);

        // Serialize input to bytes. This is where I: Serialize helps.
        // If I is Vec<u8>, it's just input.as_bytes().
        // For other I, you'd use bincode, serde_json, etc.
        // For now, assuming I is Vec<u8> or similar for direct write.
        // If I must be generic, this needs more thought.
        // Let's stick to `input.as_bytes()` which `Input` trait provides.
        let mut file = File::create(&file_path).map_err(|e| {
            CorpusError::IoError(format!("Failed to create input file {file_path:?}: {e}",))
        })?;
        file.write_all(input.as_bytes()).map_err(|e| {
            CorpusError::IoError(format!("Failed to write to input file {file_path:?}: {e}",))
        })?;

        self.entries_index.push((filename_stem, on_disk_meta));
        let internal_id = self.entries_index.len() - 1; // This is the Vec index
        self.current_id += 1; // For next filename

        // TODO: Persist the updated index
        // self.save_index()?;

        Ok(internal_id) // Return the index within entries_index
    }

    fn get(&self, id: usize) -> Option<(&I, &Box<dyn Any + Send + Sync>)> {
        // This is tricky. We stored OnDiskCorpusEntryMetadata.
        // The trait returns &Box<dyn Any...>. We need to reconstruct it.
        // For MVP, `get` for OnDiskCorpus might be limited or needs careful thought on metadata.
        // If we only stored inputs, then metadata would be None or a placeholder.
        // If we store serialized metadata, we'd deserialize it here.
        //
        // Simplification for MVP: OnDiskCorpus::get only returns the input, metadata part is problematic
        // without more infrastructure for dynamic metadata serialization.
        // Let's assume for now this `get` will be less used or will have limitations.
        //
        // A more robust `get` that reads from file and reconstructs:
        if let Some((filename_stem, _meta_ref_in_index)) = self.entries_index.get(id) {
            let file_path = self.get_file_path(filename_stem);
            if let Ok(mut file) = File::open(&file_path) {
                let mut buffer = Vec::new();
                if file.read_to_end(&mut buffer).is_ok() {
                    // Now, how to get from Vec<u8> back to I if I is not Vec<u8>?
                    // This implies I needs a From<Vec<u8>> or similar, or Deserialize.
                    // The trait bounds I: Input + Serialize + for<'de> Deserialize<'de>
                    // and Input requiring as_bytes means we probably assume I can be reconstructed.
                    // If I is just Vec<u8>, this is easy.
                    // For this example, let's assume I = Vec<u8> and `from_vec` exists or I is `From<Vec<u8>>`.
                    // This is a simplification!
                    //
                    // This part is highly dependent on how I is defined and if it can be created from Vec<u8>.
                    // If I = Vec<u8>, then it's just buffer.
                    // If I is generic, we need a way to construct it.
                    // The trait bounds `for<'de> Deserialize<'de>` on `I` mean we *could* deserialize it.
                    // Let's assume for now we store it raw and can deserialize if not Vec<u8>.
                    //
                    // This is a placeholder for the actual deserialization.
                    // If I is Vec<u8>, then I::from(buffer) would work if From<Vec<u8>> for I is impl'd.
                    // For this example, we CANNOT easily return &I because we just read it.
                    // The Corpus trait signature `fn get(&self, ...) -> Option<(&I, ...)>` is problematic for on-disk.
                    // It implies the corpus holds owned `I` instances in memory.
                    //
                    // This highlights a design challenge for OnDiskCorpus with the current Corpus trait.
                    // For now, this `get` will be largely unimplemented or panic for OnDisk.
                    // A real OnDiskCorpus might load-on-demand and cache, or have a different trait.
                    //
                    // Let's make it panic for now to highlight this needs a proper solution.
                    // A common solution is that `get` returns an owned `I`, or `Corpus` is generic over `&'a I`.
                    // Or, the primary use of OnDiskCorpus is via `random_select_path` and then loading from path.
                    //
                    // Given current trait:
                    // This is basically impossible to implement correctly for OnDiskCorpus to return &I
                    // unless we load everything into memory, defeating the purpose.
                    // For now, this will effectively not work for OnDiskCorpus.
                    // We will focus on random_select returning a path.
                    // A "get by ID" would typically return an owned input.
                    // We'll return None to indicate the limitation for now with current trait.
                    return None; // Placeholder, see comments above.
                }
            }
        }
        None
    }

    fn random_select(
        &self,
        _rng: &mut dyn RngCore,
    ) -> Option<(usize, &I, &Box<dyn Any + Send + Sync>)> {
        // Similar issue to get(): returning &I is hard if it's read from disk on demand.
        // This method also needs rethinking for an OnDiskCorpus if we don't load all into memory.
        //
        // Common strategy for on-disk:
        // 1. List all files in self.corpus_dir.
        // 2. Pick one randomly.
        // 3. Load it from disk (returns owned I).
        // This doesn't fit the `&I` return type.
        //
        // For now, like get(), this will be problematic.
        // Let's assume the `entries_index` is small enough to be in memory, but the actual inputs `I`
        // are on disk.
        //
        // To adhere to the trait for now, this would imply loading a random input into some temporary
        // owned storage within the OnDiskCorpus struct just to return a reference, which is bad.
        //
        // Let's make this also largely non-functional for now, pointing to the trait design issue
        // for on-disk scenarios.
        if self.entries_index.is_empty() {
            return None;
        }
        // This would select from the index, but then we can't return &I and &Box<dyn Any>
        // that live only for the scope of this function after reading from disk.
        // let idx = rng.next_u32() as usize % self.entries_index.len();
        // ... then load from disk ...
        None // Placeholder
    }

    fn len(&self) -> usize {
        self.entries_index.len()
        // Or, more accurately for on-disk: count files in self.corpus_dir.
        // For now, based on our in-memory index of what we've added.
    }
}

#[cfg(test)]
mod on_disk_corpus_tests {
    use super::*;
    use rand_chacha::ChaCha8Rng;
    use rand_core::SeedableRng;
    use tempfile::tempdir; // For creating temporary directories for tests

    // Simple metadata for testing OnDiskCorpus
    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
    struct DiskMeta {
        desc: String,
    }

    #[test]
    fn on_disk_corpus_create_and_add() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?; // Create a temp directory for this test
        let mut corpus: OnDiskCorpus<Vec<u8>> = OnDiskCorpus::new(dir.path().to_path_buf())?;

        let input1_data: Vec<u8> = vec![1, 2, 3, 4, 5];
        // For OnDiskCorpus, the metadata passed to add() should ideally be what it expects to store,
        // or it creates its own.
        let meta1: Box<dyn Any + Send + Sync> = Box::new(OnDiskCorpusEntryMetadata {
            source_description: "seed1".to_string(),
        });
        let id1 = corpus.add(input1_data.clone(), meta1)?;
        assert_eq!(id1, 0);
        assert_eq!(corpus.len(), 1);

        // Verify file was created
        let expected_file_path = dir.path().join("id_000000.input");
        assert!(expected_file_path.exists());
        let content = fs::read(expected_file_path)?;
        assert_eq!(content, input1_data);

        let input2_data: Vec<u8> = vec![6, 7, 8];
        let meta2: Box<dyn Any + Send + Sync> = Box::new("seed2_as_string_meta".to_string());
        let id2 = corpus.add(input2_data.clone(), meta2)?;
        assert_eq!(id2, 1);
        assert_eq!(corpus.len(), 2);

        let expected_file_path2 = dir.path().join("id_000001.input");
        assert!(expected_file_path2.exists());
        let content2 = fs::read(expected_file_path2)?;
        assert_eq!(content2, input2_data);

        // Test `get` and `random_select` limitations for OnDiskCorpus
        // For now, they return None as per current simplified impl.
        let mut rng = ChaCha8Rng::from_seed([0; 32]);
        assert!(
            corpus.get(0).is_none(),
            "OnDiskCorpus::get is currently limited"
        );
        assert!(
            corpus.random_select(&mut rng).is_none(),
            "OnDiskCorpus::random_select is currently limited"
        );

        dir.close()?; // Clean up temp directory
        Ok(())
    }

    #[test]
    fn on_disk_corpus_new_dir_creation() -> Result<(), Box<dyn std::error::Error>> {
        let base_temp_dir = tempdir()?;
        let new_corpus_dir = base_temp_dir.path().join("new_corpus_sub_dir");

        assert!(!new_corpus_dir.exists());
        let _corpus: OnDiskCorpus<Vec<u8>> = OnDiskCorpus::new(new_corpus_dir.clone())?;
        assert!(new_corpus_dir.exists());
        assert!(new_corpus_dir.is_dir());

        base_temp_dir.close()?;
        Ok(())
    }

    #[test]
    fn on_disk_corpus_path_is_file_error() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let file_path = dir.path().join("iam_a_file.txt");
        File::create(&file_path)?; // Create a file

        let result = OnDiskCorpus::<Vec<u8>>::new(file_path);
        assert!(result.is_err());
        if let Err(CorpusError::IoError(msg)) = result {
            assert!(msg.contains("not a directory"));
        } else {
            panic!("Expected IoError for non-directory path");
        }
        dir.close()?;
        Ok(())
    }
}

#[cfg(test)]
mod in_memory_corpus_test {
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
