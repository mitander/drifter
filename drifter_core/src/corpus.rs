use crate::input::Input;
use bincode::{
    self, config as bincode_config,
    de::Decode,
    enc::Encode,
    error::{DecodeError, EncodeError},
};
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::marker::PhantomData;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CorpusError {
    #[error("Input ID {0} not found in corpus or index")]
    NotFound(usize),
    #[error("Corpus is empty, cannot select an input")]
    Empty,
    #[error("Corpus I/O error: {0}")]
    IoError(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
    #[error("Input deserialization error: {0}")]
    InputDeserializationError(String),
}

impl From<std::io::Error> for CorpusError {
    fn from(err: std::io::Error) -> Self {
        CorpusError::IoError(err.to_string())
    }
}
impl From<serde_json::Error> for CorpusError {
    fn from(err: serde_json::Error) -> Self {
        CorpusError::DeserializationError(format!("JSON error: {}", err))
    }
}
impl From<EncodeError> for CorpusError {
    fn from(err: EncodeError) -> Self {
        CorpusError::SerializationError(format!("Bincode EncodeError: {}", err))
    }
}
impl From<DecodeError> for CorpusError {
    fn from(err: DecodeError) -> Self {
        CorpusError::DeserializationError(format!("Bincode DecodeError: {}", err))
    }
}

pub trait Corpus<I: Input>: Send + Sync {
    fn add(&mut self, input: I, metadata: Box<dyn Any + Send + Sync>)
    -> Result<usize, CorpusError>;
    fn get(&mut self, id: usize) -> Option<(&I, &Box<dyn Any + Send + Sync>)>;
    fn random_select(
        &mut self,
        rng: &mut dyn RngCore,
    ) -> Option<(usize, &I, &Box<dyn Any + Send + Sync>)>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn load_initial_seeds(&mut self, seed_paths: &[PathBuf]) -> Result<usize, CorpusError>;
}

#[derive(Default)]
pub struct InMemoryCorpus<I: Input> {
    entries: Vec<(I, Box<dyn Any + Send + Sync>)>,
    _marker: PhantomData<I>,
}

impl<I: Input> InMemoryCorpus<I> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<I: Input + From<Vec<u8>>> Corpus<I> for InMemoryCorpus<I> {
    fn add(
        &mut self,
        input: I,
        metadata: Box<dyn Any + Send + Sync>,
    ) -> Result<usize, CorpusError> {
        let id = self.entries.len();
        self.entries.push((input, metadata));
        Ok(id)
    }

    fn get(&mut self, id: usize) -> Option<(&I, &Box<dyn Any + Send + Sync>)> {
        self.entries.get(id).map(|(inp, meta)| (inp, meta))
    }

    fn random_select(
        &mut self,
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

    fn load_initial_seeds(&mut self, seed_paths: &[PathBuf]) -> Result<usize, CorpusError> {
        let mut count = 0;
        for path in seed_paths {
            if path.is_file() {
                let data_bytes = fs::read(path)?;
                let input_obj = I::from(data_bytes);
                let meta: Box<dyn Any + Send + Sync> = Box::new(format!("Seed: {:?}", path));
                self.add(input_obj, meta)?;
                count += 1;
            } else if path.is_dir() {
                for entry in fs::read_dir(path)? {
                    let entry = entry?;
                    let file_path = entry.path();
                    if file_path.is_file() {
                        let data_bytes = fs::read(&file_path)?;
                        let input_obj = I::from(data_bytes);
                        let meta: Box<dyn Any + Send + Sync> =
                            Box::new(format!("Seed Dir: {:?}", file_path));
                        self.add(input_obj, meta)?;
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OnDiskCorpusEntryMetadata {
    pub source_description: String,
}

#[derive(Debug)]
struct OnDiskCorpusCache<I: Input> {
    id: usize,
    input: I,
    metadata_wrapper: Box<dyn Any + Send + Sync>,
}

#[derive(Debug)]
pub struct OnDiskCorpus<I: Input + Serialize + for<'de> Deserialize<'de> + Encode + Decode<()>> {
    corpus_dir: PathBuf,
    index_file_path: PathBuf,
    entries_index: HashMap<String, OnDiskCorpusEntryMetadata>,
    id_to_stem: Vec<String>,
    cache: Option<OnDiskCorpusCache<I>>,
    _input_marker: PhantomData<I>,
}

impl<I: Input + Serialize + for<'de> Deserialize<'de> + Encode + Decode<()>> OnDiskCorpus<I> {
    const INDEX_FILENAME: &'static str = "corpus_index.json";
    const INPUT_FILE_EXTENSION: &'static str = "fuzzinput";

    pub fn new(corpus_dir: PathBuf) -> Result<Self, CorpusError> {
        if !corpus_dir.exists() {
            fs::create_dir_all(&corpus_dir)?;
        } else if !corpus_dir.is_dir() {
            return Err(CorpusError::IoError(format!(
                "Corpus path {:?} is not a directory",
                corpus_dir
            )));
        }
        let index_file_path = corpus_dir.join(Self::INDEX_FILENAME);
        let mut s = Self {
            corpus_dir,
            index_file_path,
            entries_index: HashMap::new(),
            id_to_stem: Vec::new(),
            cache: None,
            _input_marker: PhantomData,
        };
        s.load_index_from_disk().map_err(|e| {
            CorpusError::IoError(format!("Failed to load corpus index on init: {}", e))
        })?;
        Ok(s)
    }

    fn get_file_path_from_stem(&self, filename_stem: &str) -> PathBuf {
        self.corpus_dir
            .join(filename_stem)
            .with_extension(Self::INPUT_FILE_EXTENSION)
    }

    fn save_index_to_disk(&self) -> Result<(), CorpusError> {
        let file = BufWriter::new(File::create(&self.index_file_path)?);
        let data_to_save = (&self.id_to_stem, &self.entries_index);
        serde_json::to_writer_pretty(file, &data_to_save)?;
        Ok(())
    }

    fn load_index_from_disk(&mut self) -> Result<(), CorpusError> {
        if self.index_file_path.exists() {
            let file_content = fs::read_to_string(&self.index_file_path)?;
            if file_content.trim().is_empty() {
                self.id_to_stem = Vec::new();
                self.entries_index = HashMap::new();
                return Ok(());
            }
            let (id_to_stem, entries_index) = serde_json::from_str(&file_content)?;
            self.id_to_stem = id_to_stem;
            self.entries_index = entries_index;
        } else {
            self.id_to_stem = Vec::new();
            self.entries_index = HashMap::new();
        }
        Ok(())
    }

    fn load_input_from_path(&self, file_path: &PathBuf) -> Result<I, CorpusError> {
        let file_content = fs::read(file_path)?;
        let config = bincode_config::standard();
        let (decoded_val, _len): (I, usize) = bincode::decode_from_slice(&file_content, config)?;
        Ok(decoded_val)
    }
}

impl<I: Input + Serialize + for<'de> Deserialize<'de> + Encode + Decode<()>> Corpus<I>
    for OnDiskCorpus<I>
{
    fn add(
        &mut self,
        input: I,
        metadata: Box<dyn Any + Send + Sync>,
    ) -> Result<usize, CorpusError> {
        let on_disk_meta_concrete = if let Some(s_meta) = metadata.downcast_ref::<String>() {
            OnDiskCorpusEntryMetadata {
                source_description: s_meta.clone(),
            }
        } else if let Some(od_meta) = metadata.downcast_ref::<OnDiskCorpusEntryMetadata>() {
            od_meta.clone()
        } else {
            OnDiskCorpusEntryMetadata {
                source_description: format!("entry_{}", self.id_to_stem.len()),
            }
        };

        let filename_stem = format!("id_{:06}", self.id_to_stem.len());
        let file_path = self.get_file_path_from_stem(&filename_stem);

        let config = bincode_config::standard();
        let bytes_to_write = bincode::encode_to_vec(&input, config)?;

        let mut file = File::create(&file_path)?;
        file.write_all(&bytes_to_write)?;

        self.id_to_stem.push(filename_stem.clone());
        self.entries_index
            .insert(filename_stem, on_disk_meta_concrete);

        self.save_index_to_disk()?;
        Ok(self.id_to_stem.len() - 1)
    }

    fn get(&mut self, id: usize) -> Option<(&I, &Box<dyn Any + Send + Sync>)> {
        if let Some(cached_item) = self.cache.take() {
            if cached_item.id == id {
                self.cache = Some(cached_item);
                return self.cache.as_ref().map(|c| (&c.input, &c.metadata_wrapper));
            }
        }

        let filename_stem = match self.id_to_stem.get(id) {
            Some(s) => s.clone(),
            None => return None,
        };

        let file_path = self.get_file_path_from_stem(&filename_stem);
        match self.entries_index.get(&filename_stem) {
            Some(persisted_meta_concrete) => match self.load_input_from_path(&file_path) {
                Ok(input_data) => {
                    let new_metadata_wrapper: Box<dyn Any + Send + Sync> =
                        Box::new(persisted_meta_concrete.clone());

                    self.cache = Some(OnDiskCorpusCache {
                        id,
                        input: input_data,
                        metadata_wrapper: new_metadata_wrapper,
                    });
                    self.cache.as_ref().map(|c| (&c.input, &c.metadata_wrapper))
                }
                Err(e) => {
                    eprintln!("Error loading input for ID {}: {:?}", id, e);
                    None
                }
            },
            None => {
                eprintln!(
                    "Corpus index inconsistency: stem for ID {} found but no metadata.",
                    id
                );
                None
            }
        }
    }

    fn random_select(
        &mut self,
        rng: &mut dyn RngCore,
    ) -> Option<(usize, &I, &Box<dyn Any + Send + Sync>)> {
        if self.id_to_stem.is_empty() {
            self.cache = None;
            return None;
        }
        let id = rng.next_u32() as usize % self.id_to_stem.len();
        self.get(id)
            .map(|(input_ref, meta_ref)| (id, input_ref, meta_ref))
    }

    fn len(&self) -> usize {
        self.id_to_stem.len()
    }

    fn load_initial_seeds(&mut self, seed_paths: &[PathBuf]) -> Result<usize, CorpusError> {
        let mut count = 0;
        for path in seed_paths {
            if path.is_file() {
                let input_obj = self.load_input_from_path(path)?;
                let meta_desc = format!("Initial Seed File: {:?}", path);
                let on_disk_meta = OnDiskCorpusEntryMetadata {
                    source_description: meta_desc,
                };
                let meta_any: Box<dyn Any + Send + Sync> = Box::new(on_disk_meta);
                self.add(input_obj, meta_any)?;
                count += 1;
            } else if path.is_dir() {
                for entry in fs::read_dir(path)? {
                    let entry = entry?;
                    let file_path = entry.path();
                    if file_path.is_file() {
                        let input_obj = self.load_input_from_path(&file_path)?;
                        let meta_desc = format!("Initial Seed Dir: {:?}", file_path);
                        let on_disk_meta = OnDiskCorpusEntryMetadata {
                            source_description: meta_desc,
                        };
                        let meta_any: Box<dyn Any + Send + Sync> = Box::new(on_disk_meta);
                        self.add(input_obj, meta_any)?;
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod on_disk_corpus_tests {
    use super::*;
    use bincode::{Decode, Encode};
    use rand_chacha::ChaCha8Rng;
    use rand_core::SeedableRng;
    use tempfile::tempdir;

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Encode, Decode)]
    struct TestInputStruct {
        data: Vec<u8>,
        id_num: u32,
    }
    impl Input for TestInputStruct {
        fn as_bytes(&self) -> &[u8] {
            &self.data
        }
        fn len(&self) -> usize {
            self.data.len()
        }
        fn is_empty(&self) -> bool {
            self.data.is_empty()
        }
    }

    impl From<Vec<u8>> for TestInputStruct {
        fn from(vec: Vec<u8>) -> Self {
            let config = bincode_config::standard();
            bincode::decode_from_slice(&vec, config).map(|(val, _)| val).unwrap_or_else(|e| {
                eprintln!("Warning: Failed to deserialize TestInputStruct in From<Vec<u8>>: {:?}, using default.", e);
                TestInputStruct { data: vec, id_num: 0 }
            })
        }
    }

    #[test]
    fn on_disk_corpus_vec_u8() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let mut corpus: OnDiskCorpus<Vec<u8>> = OnDiskCorpus::new(dir.path().to_path_buf())?;

        let input1_data: Vec<u8> = vec![1, 2, 3, 4, 5];
        let meta1_any: Box<dyn Any + Send + Sync> = Box::new("seed1_vec_u8".to_string());

        let id1 = corpus.add(input1_data.clone(), meta1_any)?;
        assert_eq!(corpus.len(), 1);

        let expected_file_path = dir.path().join("id_000000.fuzzinput");
        assert!(expected_file_path.exists());
        let file_content = fs::read(expected_file_path)?;
        let config = bincode_config::standard();
        let (deserialized_input, _len): (Vec<u8>, usize) =
            bincode::decode_from_slice(&file_content, config)?;
        assert_eq!(deserialized_input, input1_data);

        let mut rng = ChaCha8Rng::from_seed([0; 32]);
        if let Some((selected_id, selected_input, selected_meta_wrapper)) =
            corpus.random_select(&mut rng)
        {
            assert_eq!(selected_id, id1);
            assert_eq!(selected_input, &input1_data);
            let meta_concrete = selected_meta_wrapper
                .downcast_ref::<OnDiskCorpusEntryMetadata>()
                .unwrap();
            assert_eq!(meta_concrete.source_description, "seed1_vec_u8");
        } else {
            panic!("random_select failed for Vec<u8>");
        }

        if let Some((get_input, get_meta_wrapper)) = corpus.get(id1) {
            assert_eq!(get_input, &input1_data);
            let meta_concrete = get_meta_wrapper
                .downcast_ref::<OnDiskCorpusEntryMetadata>()
                .unwrap();
            assert_eq!(meta_concrete.source_description, "seed1_vec_u8");
        } else {
            panic!("get(0) failed for Vec<u8>");
        }

        dir.close()?;
        Ok(())
    }

    #[test]
    fn on_disk_corpus_custom_struct() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let mut corpus: OnDiskCorpus<TestInputStruct> =
            OnDiskCorpus::new(dir.path().to_path_buf())?;

        let input1_data = TestInputStruct {
            data: vec![10, 20],
            id_num: 1,
        };
        let meta1_any: Box<dyn Any + Send + Sync> = Box::new("seed1_struct".to_string());

        let _id1 = corpus.add(input1_data.clone(), meta1_any)?;
        assert_eq!(corpus.len(), 1);

        let expected_file_path = dir.path().join("id_000000.fuzzinput");
        assert!(expected_file_path.exists());
        let file_content = fs::read(expected_file_path)?;
        let config = bincode_config::standard();
        let (deserialized_input, _len): (TestInputStruct, usize) =
            bincode::decode_from_slice(&file_content, config)?;
        assert_eq!(deserialized_input, input1_data);

        let mut rng = ChaCha8Rng::from_seed([0; 32]);
        if let Some((_id, selected_input, _meta)) = corpus.random_select(&mut rng) {
            assert_eq!(selected_input, &input1_data);
        } else {
            panic!("random_select failed for TestInputStruct");
        }
        dir.close()?;
        Ok(())
    }

    #[test]
    fn on_disk_corpus_index_persistence() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let corpus_path = dir.path().to_path_buf();
        let input1_data: Vec<u8> = vec![1, 2, 3];
        let meta1_str = "entry_one".to_string();

        {
            let mut corpus: OnDiskCorpus<Vec<u8>> = OnDiskCorpus::new(corpus_path.clone())?;
            assert_eq!(corpus.len(), 0);
            corpus.add(input1_data.clone(), Box::new(meta1_str.clone()))?;
            assert_eq!(corpus.len(), 1);
        }

        let mut reloaded_corpus: OnDiskCorpus<Vec<u8>> = OnDiskCorpus::new(corpus_path)?;
        assert_eq!(reloaded_corpus.len(), 1, "Should load 1 entry from index");

        if let Some((input_ref, meta_wrapper_ref)) = reloaded_corpus.get(0) {
            assert_eq!(input_ref, &input1_data);
            let meta_concrete = meta_wrapper_ref
                .downcast_ref::<OnDiskCorpusEntryMetadata>()
                .unwrap();
            assert_eq!(meta_concrete.source_description, meta1_str);
        } else {
            panic!("Failed to get entry from reloaded corpus");
        }
        dir.close()?;
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
        File::create(&file_path)?;

        let result = OnDiskCorpus::<Vec<u8>>::new(file_path);
        assert!(result.is_err());
        if let Err(CorpusError::IoError(msg)) = result {
            assert!(msg.contains("not a directory"));
        } else {
            panic!("Expected IoError for non-directory path, got {:?}", result);
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

    #[derive(Debug, PartialEq, Eq, Clone)]
    struct TestMetadata {
        info: String,
    }

    #[test]
    fn in_memory_corpus_add_and_get() {
        let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
        let input1 = vec![1, 2, 3];
        let meta1: Box<dyn Any + Send + Sync> = Box::new(TestMetadata {
            info: "first".to_string(),
        });
        let id1 = corpus.add(input1.clone(), meta1).unwrap();
        assert_eq!(id1, 0);

        let input2 = vec![4, 5, 6];
        let meta2: Box<dyn Any + Send + Sync> = Box::new(TestMetadata {
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

#[cfg(test)]
mod dummy_corpus_for_mutator_tests {
    use super::*;

    pub struct DummyCorpusForMutator;
    impl<I: Input> Corpus<I> for DummyCorpusForMutator {
        fn add(
            &mut self,
            _input: I,
            _metadata: Box<dyn Any + Send + Sync>,
        ) -> Result<usize, CorpusError> {
            Ok(0)
        }
        fn get(&mut self, _id: usize) -> Option<(&I, &Box<dyn Any + Send + Sync>)> {
            None
        }
        fn random_select(
            &mut self,
            _rng: &mut dyn RngCore,
        ) -> Option<(usize, &I, &Box<dyn Any + Send + Sync>)> {
            None
        }
        fn len(&self) -> usize {
            0
        }
        fn load_initial_seeds(&mut self, _seed_paths: &[PathBuf]) -> Result<usize, CorpusError> {
            Ok(0)
        }
    }
}
