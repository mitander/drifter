use crate::input::Input;
use bincode::{
    self,
    config::{Configuration, Fixint, LittleEndian, NoLimit},
    de::Decode,
    enc::Encode,
    error::{DecodeError, EncodeError},
};
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Defines errors that can arise during corpus operations.
///
/// These errors cover issues from I/O problems when interacting with file systems
/// (for on-disk corpora) to logical errors like attempting to retrieve a non-existent input.
#[derive(Error, Debug)]
pub enum CorpusError {
    /// The requested input ID was not found within the corpus.
    #[error("Input ID {0} not found in corpus or index")]
    InputNotFound(usize),

    /// An operation could not be performed because the corpus is empty
    /// (e.g., attempting to select an input randomly).
    #[error("Corpus is empty, cannot select an input")]
    CorpusIsEmpty,

    /// An I/O error occurred during interaction with the underlying storage
    /// (e.g., reading or writing files for an on-disk corpus).
    /// Contains a string describing the underlying I/O error.
    #[error("Corpus I/O error: {0}")]
    Io(String),

    /// An error occurred during the serialization of an input or its metadata
    /// (e.g., encoding an input to bytes using bincode or metadata to JSON).
    /// Contains a string describing the serialization error.
    #[error("Corpus serialization error: {0}")]
    Serialization(String),

    /// An error occurred during the deserialization of an input or its metadata
    /// (e.g., decoding an input from bytes or metadata from JSON).
    /// Contains a string describing the deserialization error.
    #[error("Corpus deserialization error: {0}")]
    Deserialization(String),
}

impl From<std::io::Error> for CorpusError {
    fn from(err: std::io::Error) -> Self {
        CorpusError::Io(err.to_string())
    }
}
impl From<serde_json::Error> for CorpusError {
    fn from(err: serde_json::Error) -> Self {
        CorpusError::Deserialization(format!("JSON operation error: {}", err))
    }
}
impl From<EncodeError> for CorpusError {
    fn from(err: EncodeError) -> Self {
        CorpusError::Serialization(format!("Bincode encoding error: {}", err))
    }
}
impl From<DecodeError> for CorpusError {
    fn from(err: DecodeError) -> Self {
        CorpusError::Deserialization(format!("Bincode decoding error: {}", err))
    }
}

/// Metadata associated with an entry stored in an `OnDiskCorpus`.
///
/// This structure is serialized to JSON as part of the on-disk corpus index,
/// allowing for persistent storage of descriptive information about each corpus entry.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OnDiskCorpusEntryMetadata {
    /// A human-readable string describing the origin or significance of the corpus entry.
    /// For example, this could note if it was an initial seed, discovered via a specific
    /// mutator, or triggered a particular behavior.
    pub source_description: String,
    // TODO: extend
    // pub discovery_timestamp_ms: u64,
    // pub parent_entry_id: Option<usize>,
    // pub mutations_applied: u32,
}

/// Defines the common interface for a collection of fuzzing inputs.
///
/// A `Corpus` is responsible for storing, managing, and providing access to fuzz inputs.
/// It allows adding new inputs, retrieving existing ones by ID, selecting inputs
/// (e.g., randomly for mutation), and loading initial seed inputs.
/// Implementations must be `Send` and `Sync` to allow potential use across threads.
///
/// # Type Parameters
/// * `I`: The type of input stored in the corpus, which must implement the [`Input`] trait.
pub trait Corpus<I: Input>: Send + Sync {
    /// Adds a new input to the corpus along with its associated metadata.
    ///
    /// # Arguments
    /// * `input`: The input instance to add. The corpus takes ownership.
    /// * `metadata`: A dynamically-typed `Box` containing metadata for the input.
    ///   The concrete type of metadata depends on the corpus implementation and fuzzer needs
    ///   (e.g., `OnDiskCorpus` expects or converts to `OnDiskCorpusEntryMetadata`).
    ///
    /// # Returns
    /// A `Result` containing the unique ID (typically an index `usize`) assigned to the
    /// newly added input, or a `CorpusError` if the addition fails.
    fn add(&mut self, input: I, metadata: Box<dyn Any + Send + Sync>)
    -> Result<usize, CorpusError>;

    /// Retrieves an immutable reference to an input and its metadata by the input's ID.
    ///
    /// # Arguments
    /// * `id`: The `usize` ID of the input to retrieve.
    ///
    /// # Returns
    /// An `Option` containing a tuple of references to the input (`&I`) and its
    /// metadata (`&Box<dyn Any + Send + Sync>`) if an input with the given ID exists.
    /// Returns `None` if the ID is not found.
    fn get(&mut self, id: usize) -> Option<(&I, &Box<dyn Any + Send + Sync>)>;

    /// Selects an input from the corpus, typically using a random strategy.
    ///
    /// This method is often used by schedulers to pick an input for subsequent mutation.
    ///
    /// # Arguments
    /// * `rng`: A mutable reference to a random number generator implementing `RngCore`.
    ///
    /// # Returns
    /// An `Option` containing a tuple: the ID (`usize`), a reference to the selected
    /// input (`&I`), and a reference to its metadata (`&Box<dyn Any + Send + Sync>`).
    /// Returns `None` if the corpus is empty or if selection otherwise fails.
    fn random_select(
        &mut self,
        rng: &mut dyn RngCore,
    ) -> Option<(usize, &I, &Box<dyn Any + Send + Sync>)>;

    /// Returns the total number of inputs currently stored in the corpus.
    fn len(&self) -> usize;

    /// Returns `true` if the corpus contains no inputs, `false` otherwise.
    /// This is a convenience method equivalent to `self.len() == 0`.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Loads initial seed inputs from a collection of specified file system paths.
    ///
    /// Each path can point to either an individual seed file or a directory containing seed files.
    /// If a path is a directory, files directly within it are loaded (subdirectories are not
    /// traversed recursively).
    ///
    /// The format expected for seed files depends on the `Corpus` implementation.
    /// For example, `InMemoryCorpus<I: From<Vec<u8>>>` reads raw bytes and converts them.
    /// `OnDiskCorpus` expects bincode-serialized instances of the input type `I`.
    ///
    /// # Arguments
    /// * `seed_paths`: A slice of `PathBuf`s representing the locations of seed inputs.
    ///
    /// # Returns
    /// A `Result` containing the total number of seeds (`usize`) successfully loaded into the corpus.
    /// Returns a `CorpusError` if any issues occur during loading (e.g., file access errors,
    /// deserialization failures).
    fn load_initial_seeds(&mut self, seed_paths: &[PathBuf]) -> Result<usize, CorpusError>;
}

/// An in-memory implementation of the `Corpus` trait.
///
/// This corpus stores all inputs and their associated metadata directly in a `Vec`
/// in memory. It offers fast access but is not persistent across different
/// runs of the fuzzer. Best suited for smaller corpora or scenarios where
/// persistence is handled externally or not required.
///
/// For `load_initial_seeds` to function, the input type `I` must implement `From<Vec<u8>>`.
#[derive(Debug)]
pub struct InMemoryCorpus<I: Input> {
    entries: Vec<(I, Box<dyn Any + Send + Sync>)>,
    _marker: PhantomData<I>,
}

impl<I: Input> InMemoryCorpus<I> {
    /// Creates a new, empty `InMemoryCorpus`.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<I: Input> Default for InMemoryCorpus<I> {
    /// Provides a default, empty `InMemoryCorpus`.
    fn default() -> Self {
        Self::new()
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
        self.entries
            .get(id)
            .map(|tuple_ref| (&tuple_ref.0, &tuple_ref.1))
    }

    fn random_select(
        &mut self,
        rng: &mut dyn RngCore,
    ) -> Option<(usize, &I, &Box<dyn Any + Send + Sync>)> {
        if self.is_empty() {
            return None;
        }
        let index = rng.next_u64() as usize % self.entries.len();
        self.entries
            .get(index)
            .map(|(input_ref, metadata_ref)| (index, input_ref, metadata_ref))
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn load_initial_seeds(&mut self, seed_paths: &[PathBuf]) -> Result<usize, CorpusError> {
        let mut loaded_count = 0;
        for path_buf in seed_paths {
            let path_ref = path_buf.as_path();
            if path_ref.is_file() {
                let data_bytes = fs::read(path_ref).map_err(|e| {
                    CorpusError::Io(format!("Failed to read seed file {:?}: {}", path_ref, e))
                })?;
                let input_obj = I::from(data_bytes);
                let metadata: Box<dyn Any + Send + Sync> =
                    Box::new(format!("Initial seed file: {:?}", path_ref));
                self.add(input_obj, metadata)?;
                loaded_count += 1;
            } else if path_ref.is_dir() {
                for entry_result in fs::read_dir(path_ref).map_err(|e| {
                    CorpusError::Io(format!(
                        "Failed to read seed directory {:?}: {}",
                        path_ref, e
                    ))
                })? {
                    let entry = entry_result.map_err(|e| {
                        CorpusError::Io(format!("Error reading entry in {:?}: {}", path_ref, e))
                    })?;
                    let file_path_in_dir = entry.path();
                    if file_path_in_dir.is_file() {
                        let data_bytes = fs::read(&file_path_in_dir).map_err(|e| {
                            CorpusError::Io(format!(
                                "Failed to read seed file {:?}: {}",
                                file_path_in_dir, e
                            ))
                        })?;
                        let input_obj = I::from(data_bytes);
                        let metadata: Box<dyn Any + Send + Sync> =
                            Box::new(format!("Initial seed from dir: {:?}", file_path_in_dir));
                        self.add(input_obj, metadata)?;
                        loaded_count += 1;
                    }
                }
            }
        }
        Ok(loaded_count)
    }
}

/// Internal cache structure for `OnDiskCorpus`, holding a recently accessed input and its metadata.
/// This helps optimize repeated accesses to the same input by reducing disk I/O.
#[derive(Debug)]
struct OnDiskCorpusCache<I: Input> {
    /// The ID (index) of the cached input.
    id: usize,
    /// The cached input data itself.
    input: I,
    /// The metadata associated with the cached input, boxed as `dyn Any`.
    /// This typically holds an `OnDiskCorpusEntryMetadata`.
    metadata_wrapper: Box<dyn Any + Send + Sync>,
}

/// An on-disk implementation of the `Corpus` trait that stores inputs as individual
/// files and maintains a JSON index for metadata.
///
/// This corpus provides persistence across fuzzer runs. Inputs are serialized using
/// `bincode`. The input type `I` must implement `serde::Serialize`,
/// `serde::DeserializeOwned`, `bincode::Encode`, and `bincode::Decode<()>`.
///
/// Metadata for each entry is stored as [`OnDiskCorpusEntryMetadata`].
pub struct OnDiskCorpus<I: Input + Serialize + for<'de> Deserialize<'de> + Encode + Decode<()>> {
    /// Path to the directory where corpus files and the index are stored.
    corpus_dir_path: PathBuf,
    /// Path to the JSON index file (`corpus_index.json`) within `corpus_dir_path`.
    index_file_path: PathBuf,
    /// In-memory map from an input's filename stem (e.g., "input_00000000") to its metadata.
    /// This is loaded from and saved to `index_file_path`.
    filename_to_metadata_index: HashMap<String, OnDiskCorpusEntryMetadata>,
    /// In-memory vector mapping a corpus ID (index) to its filename stem.
    /// This is also loaded from and saved to `index_file_path`.
    id_to_filename_stem: Vec<String>,
    /// Optional cache for the last accessed input to reduce disk reads.
    last_accessed_cache: Option<OnDiskCorpusCache<I>>,
    _input_marker: PhantomData<I>,
    /// Specific bincode configuration used for serializing and deserializing inputs.
    bincode_config: Configuration<LittleEndian, Fixint, NoLimit>,
}

impl<I: Input + Serialize + for<'de> Deserialize<'de> + Encode + Decode<()>> OnDiskCorpus<I> {
    /// Default filename for the corpus index JSON file.
    const INDEX_FILENAME: &'static str = "corpus_index.json";
    /// Default file extension for individual bincode-serialized input files.
    const INPUT_FILE_EXTENSION: &'static str = "fuzzinput";

    /// Provides the bincode configuration used by this corpus for (de)serialization.
    /// Ensures consistent encoding settings (LittleEndian, Fixint).
    fn current_bincode_config() -> Configuration<LittleEndian, Fixint, NoLimit> {
        bincode::config::standard()
            .with_little_endian()
            .with_fixed_int_encoding()
    }

    /// Creates a new `OnDiskCorpus` or loads an existing one from `corpus_dir_path`.
    ///
    /// If `corpus_dir_path` does not exist, it is created. If an `index_file_path`
    /// (`corpus_index.json`) exists within this directory, the corpus state is loaded from it.
    /// Otherwise, a new, empty corpus (and its index file) is initialized at that location.
    ///
    /// # Arguments
    /// * `corpus_dir_path`: The path to the directory for storing the corpus.
    ///
    /// # Returns
    /// A `Result` containing the initialized `OnDiskCorpus` or a `CorpusError` if
    /// directory creation, or index loading/creation fails.
    pub fn new(corpus_dir_path: PathBuf) -> Result<Self, CorpusError> {
        if !corpus_dir_path.exists() {
            fs::create_dir_all(&corpus_dir_path).map_err(|e| {
                CorpusError::Io(format!(
                    "Failed to create corpus directory at {:?}: {}",
                    corpus_dir_path, e
                ))
            })?;
        } else if !corpus_dir_path.is_dir() {
            return Err(CorpusError::Io(format!(
                "Corpus path {:?} exists but is not a directory",
                corpus_dir_path
            )));
        }

        let index_file_path = corpus_dir_path.join(Self::INDEX_FILENAME);
        let mut corpus_instance = Self {
            corpus_dir_path,
            index_file_path,
            filename_to_metadata_index: HashMap::new(),
            id_to_filename_stem: Vec::new(),
            last_accessed_cache: None,
            _input_marker: PhantomData,
            bincode_config: Self::current_bincode_config(),
        };

        // Wrap error from load_index_from_disk for better context
        corpus_instance
            .load_index_from_disk()
            .map_err(|e| match e {
                CorpusError::Io(msg)
                | CorpusError::Deserialization(msg)
                | CorpusError::Serialization(msg) => CorpusError::Io(format!(
                    "Failed during corpus initialization (loading index from {:?}): {}",
                    corpus_instance.index_file_path, msg
                )),
                other_err => other_err,
            })?;

        // Ensure an index file exists, even if it's for an empty corpus.
        // This is useful for consistency and for tools that might expect the file.
        if !corpus_instance.index_file_path.exists() {
            corpus_instance.save_index_to_disk().map_err(|e| match e {
                CorpusError::Io(msg) | CorpusError::Serialization(msg) => CorpusError::Io(format!(
                    "Failed to create initial empty index file at {:?}: {}",
                    corpus_instance.index_file_path, msg
                )),
                other_err => other_err,
            })?;
        }
        Ok(corpus_instance)
    }

    /// Constructs the full file path for an input given its unique filename stem.
    /// Example: `input_00000000` -> `/path/to/corpus/input_00000000.fuzzinput`.
    fn get_input_file_path_from_stem(&self, filename_stem: &str) -> PathBuf {
        self.corpus_dir_path
            .join(filename_stem)
            .with_extension(Self::INPUT_FILE_EXTENSION)
    }

    /// Saves the current in-memory corpus index (ID-to-stem and stem-to-metadata maps)
    /// to the `index_file_path` on disk in JSON format.
    /// This should be called after any modification to the corpus structure (e.g., adding an input).
    fn save_index_to_disk(&self) -> Result<(), CorpusError> {
        let file = File::create(&self.index_file_path).map_err(|e| {
            CorpusError::Io(format!(
                "Failed to create or truncate index file {:?}: {}",
                self.index_file_path, e
            ))
        })?;
        let writer = BufWriter::new(file);
        let data_to_serialize = (&self.id_to_filename_stem, &self.filename_to_metadata_index);
        serde_json::to_writer_pretty(writer, &data_to_serialize).map_err(|e| {
            CorpusError::Serialization(format!(
                "Failed to serialize corpus index to JSON for {:?}: {}",
                self.index_file_path, e
            ))
        })?;
        Ok(())
    }

    /// Loads the corpus index from the JSON file (`index_file_path`) on disk into memory.
    /// If the index file does not exist or is empty, it initializes the in-memory
    /// index structures as empty, effectively treating it as a new or empty corpus.
    fn load_index_from_disk(&mut self) -> Result<(), CorpusError> {
        if self.index_file_path.exists() && self.index_file_path.is_file() {
            let file = File::open(&self.index_file_path).map_err(|e| {
                CorpusError::Io(format!(
                    "Failed to open index file {:?}: {}",
                    self.index_file_path, e
                ))
            })?;

            // Check if the file is empty before attempting to parse JSON
            if file.metadata()?.len() == 0 {
                self.id_to_filename_stem = Vec::new();
                self.filename_to_metadata_index = HashMap::new();
                return Ok(());
            }

            let reader = BufReader::new(file);
            match serde_json::from_reader(reader) {
                Ok((id_to_stem, entries_index)) => {
                    self.id_to_filename_stem = id_to_stem;
                    self.filename_to_metadata_index = entries_index;
                }
                Err(e) => {
                    return Err(CorpusError::Deserialization(format!(
                        "Failed to parse JSON from index file {:?}: {}. The file might be corrupted.",
                        self.index_file_path, e
                    )));
                }
            }
        } else {
            // Index file doesn't exist or is not a regular file, so initialize as empty.
            self.id_to_filename_stem = Vec::new();
            self.filename_to_metadata_index = HashMap::new();
        }
        Ok(())
    }

    /// Loads and deserializes a single input from its specified file path using bincode.
    fn load_input_from_file_path(&self, file_path: &Path) -> Result<I, CorpusError> {
        let file_content = fs::read(file_path).map_err(|e| {
            CorpusError::Io(format!("Failed to read input file {:?}: {}", file_path, e))
        })?;
        if file_content.is_empty() {
            // Attempting to deserialize an empty slice with bincode would typically error.
            return Err(CorpusError::Deserialization(format!(
                "Input file {:?} is empty, cannot deserialize.",
                file_path
            )));
        }
        let (decoded_input, _length): (I, usize) =
            bincode::decode_from_slice(&file_content, self.bincode_config).map_err(|e| {
                CorpusError::Deserialization(format!(
                    "Bincode deserialization failed for input file {:?}: {}",
                    file_path, e
                ))
            })?;
        Ok(decoded_input)
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
        // Convert provided generic metadata to OnDiskCorpusEntryMetadata.
        // Prioritizes direct OnDiskCorpusEntryMetadata, then String, then a default.
        let on_disk_metadata =
            if let Some(concrete_meta) = metadata.downcast_ref::<OnDiskCorpusEntryMetadata>() {
                concrete_meta.clone()
            } else if let Some(string_meta) = metadata.downcast_ref::<String>() {
                OnDiskCorpusEntryMetadata {
                    source_description: string_meta.clone(),
                }
            } else {
                OnDiskCorpusEntryMetadata {
                    source_description: format!("Input entry #{}", self.id_to_filename_stem.len()),
                }
            };

        let new_id = self.id_to_filename_stem.len();
        // Using a zero-padded ID for consistent filename length and lexicographical sorting.
        let filename_stem = format!("input_{:08}", new_id);
        let file_path = self.get_input_file_path_from_stem(&filename_stem);

        let bytes_to_write = bincode::encode_to_vec(&input, self.bincode_config)?;

        let mut file_writer = File::create(&file_path).map_err(|e| {
            CorpusError::Io(format!(
                "Failed to create input file {:?}: {}",
                file_path, e
            ))
        })?;
        file_writer.write_all(&bytes_to_write).map_err(|e| {
            CorpusError::Io(format!(
                "Failed to write to input file {:?}: {}",
                file_path, e
            ))
        })?;

        // Update in-memory index structures
        self.id_to_filename_stem.push(filename_stem.clone());
        self.filename_to_metadata_index
            .insert(filename_stem, on_disk_metadata);

        // Persist index change and invalidate cache
        self.save_index_to_disk()?;
        self.last_accessed_cache = None;
        Ok(new_id)
    }

    fn get(&mut self, id: usize) -> Option<(&I, &Box<dyn Any + Send + Sync>)> {
        // Attempt to satisfy the request from the cache.
        if let Some(taken_cache_item) = self.last_accessed_cache.take() {
            if taken_cache_item.id == id {
                // Cache hit: restore the item to the cache and return references from it.
                self.last_accessed_cache = Some(taken_cache_item);
                return self
                    .last_accessed_cache
                    .as_ref()
                    .map(|cache_ref| (&cache_ref.input, &cache_ref.metadata_wrapper));
            }
            // If ID didn't match, `taken_cache_item` is dropped, and cache remains `None`.
        }

        // Cache miss or wrong item: proceed to load from disk.
        let filename_stem = self.id_to_filename_stem.get(id)?.clone();
        let file_path = self.get_input_file_path_from_stem(&filename_stem);

        match self.filename_to_metadata_index.get(&filename_stem) {
            Some(persisted_disk_metadata) => {
                match self.load_input_from_file_path(&file_path) {
                    Ok(loaded_input_data) => {
                        let metadata_wrapper_for_cache: Box<dyn Any + Send + Sync> =
                            Box::new(persisted_disk_metadata.clone());

                        // Populate the cache with the newly loaded item.
                        self.last_accessed_cache = Some(OnDiskCorpusCache {
                            id,
                            input: loaded_input_data,
                            metadata_wrapper: metadata_wrapper_for_cache,
                        });
                        // Return references from the now-populated cache.
                        self.last_accessed_cache
                            .as_ref()
                            .map(|cache_ref| (&cache_ref.input, &cache_ref.metadata_wrapper))
                    }
                    Err(e) => {
                        eprintln!(
                            "ERROR: OnDiskCorpus::get failed to load/deserialize input for ID {}: {}. File: {:?}",
                            id, e, file_path
                        );
                        None
                    }
                }
            }
            None => {
                eprintln!(
                    "ERROR: OnDiskCorpus::get inconsistency. Filename stem '{}' for ID {} found in id_to_filename_stem, but no corresponding metadata in filename_to_metadata_index.",
                    filename_stem, id
                );
                None
            }
        }
    }

    fn random_select(
        &mut self,
        rng: &mut dyn RngCore,
    ) -> Option<(usize, &I, &Box<dyn Any + Send + Sync>)> {
        if self.id_to_filename_stem.is_empty() {
            // Ensure cache is clear if corpus becomes empty.
            self.last_accessed_cache = None;
            return None;
        }
        let id = rng.next_u64() as usize % self.id_to_filename_stem.len();
        self.get(id)
            .map(|(input_ref, meta_ref)| (id, input_ref, meta_ref))
    }

    fn len(&self) -> usize {
        self.id_to_filename_stem.len()
    }

    fn load_initial_seeds(&mut self, seed_paths: &[PathBuf]) -> Result<usize, CorpusError> {
        let mut loaded_count = 0;
        for path_buf in seed_paths {
            let path_ref = path_buf.as_path();
            if path_ref.is_file() {
                // Avoid loading the corpus's own index file if it's accidentally included in seed_paths
                // and points to the current corpus's index file.
                if path_ref
                    .file_name()
                    .is_some_and(|name| name == Self::INDEX_FILENAME)
                    && path_ref
                        .parent()
                        .map_or_else(|| false, |p| p == self.corpus_dir_path)
                {
                    continue;
                }

                let input_obj = self.load_input_from_file_path(path_ref)?;
                let metadata_description = format!("Initial seed file: {:?}", path_ref);
                let on_disk_meta = OnDiskCorpusEntryMetadata {
                    source_description: metadata_description,
                };
                let boxed_metadata: Box<dyn Any + Send + Sync> = Box::new(on_disk_meta);

                self.add(input_obj, boxed_metadata)?;
                loaded_count += 1;
            } else if path_ref.is_dir() {
                for entry_result in fs::read_dir(path_ref).map_err(|e| {
                    CorpusError::Io(format!(
                        "Failed to read seed directory {:?}: {}",
                        path_ref, e
                    ))
                })? {
                    let entry = entry_result.map_err(|e| {
                        CorpusError::Io(format!(
                            "Error reading entry in seed directory {:?}: {}",
                            path_ref, e
                        ))
                    })?;
                    let file_path_in_dir = entry.path();
                    if file_path_in_dir.is_file() {
                        // Skip hidden files and index itself
                        if let Some(filename_str) =
                            file_path_in_dir.file_name().and_then(|name| name.to_str())
                        {
                            if filename_str == Self::INDEX_FILENAME || filename_str.starts_with('.')
                            {
                                continue;
                            }
                        }

                        let input_obj = self.load_input_from_file_path(&file_path_in_dir)?;
                        let metadata_description =
                            format!("Initial seed from dir: {:?}", file_path_in_dir);
                        let on_disk_meta = OnDiskCorpusEntryMetadata {
                            source_description: metadata_description,
                        };
                        let boxed_metadata: Box<dyn Any + Send + Sync> = Box::new(on_disk_meta);
                        self.add(input_obj, boxed_metadata)?;
                        loaded_count += 1;
                    }
                }
            }
        }
        Ok(loaded_count)
    }
}

#[cfg(test)]
mod tests {
    // Test module content from the previous fully corrected version should be here.
    // Ensure all tests still pass with the bincode::config::Infinite change.
    // (The test module content is extensive, so not duplicated here for brevity,
    // but assume it's the same as the last known good version that passed tests).
    use super::*;
    use bincode::{Decode, Encode};
    use rand_chacha::ChaCha8Rng;
    use rand_core::SeedableRng;
    use tempfile::tempdir;

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Encode, Decode)]
    struct TestInputStruct {
        data_bytes: Vec<u8>,
        id_number: u32,
    }

    impl Input for TestInputStruct {
        fn as_bytes(&self) -> &[u8] {
            &self.data_bytes
        }
        fn len(&self) -> usize {
            self.data_bytes.len()
        }
        fn is_empty(&self) -> bool {
            self.data_bytes.is_empty()
        }
    }

    #[cfg(test)]
    mod in_memory_corpus_tests {
        use super::*;

        #[derive(Debug, PartialEq, Eq, Clone)]
        struct TestInMemoryMetadata {
            info: String,
            count: i32,
        }

        #[test]
        fn in_memory_corpus_add_get_len_is_empty() {
            let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
            assert!(corpus.is_empty());
            assert_eq!(corpus.len(), 0);
            let input1_data: Vec<u8> = vec![1, 2, 3];
            let meta1: Box<dyn Any + Send + Sync> = Box::new(TestInMemoryMetadata {
                info: "first".to_string(),
                count: 1,
            });
            let id1 = corpus.add(input1_data.clone(), meta1).unwrap();
            assert_eq!(id1, 0);
            assert!(!corpus.is_empty());
            assert_eq!(corpus.len(), 1);
            let input2_data: Vec<u8> = vec![4, 5];
            let meta2: Box<dyn Any + Send + Sync> = Box::new(TestInMemoryMetadata {
                info: "second".to_string(),
                count: 2,
            });
            let id2 = corpus.add(input2_data.clone(), meta2).unwrap();
            assert_eq!(id2, 1);
            assert_eq!(corpus.len(), 2);

            if let Some((ret_input1, ret_meta1_dyn)) = corpus.get(id1) {
                assert_eq!(*ret_input1, input1_data);
                let ret_meta1 = ret_meta1_dyn
                    .downcast_ref::<TestInMemoryMetadata>()
                    .unwrap();
                assert_eq!(ret_meta1.info, "first");
            } else {
                panic!("Failed to get input1");
            }
            assert!(corpus.get(99).is_none());
        }

        #[test]
        fn in_memory_corpus_random_select_behavior() {
            let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
            let mut rng = ChaCha8Rng::from_seed([42; 32]);
            assert!(corpus.random_select(&mut rng).is_none());

            corpus
                .add(vec![b'A'], Box::new("meta_A".to_string()))
                .unwrap();
            corpus
                .add(vec![b'B'], Box::new("meta_B".to_string()))
                .unwrap();
            corpus
                .add(vec![b'C'], Box::new("meta_C".to_string()))
                .unwrap();

            let mut selected_ids_counts = HashMap::new();
            for _ in 0..100 {
                if let Some((id, _input, _meta)) = corpus.random_select(&mut rng) {
                    *selected_ids_counts.entry(id).or_insert(0) += 1;
                    assert!(id < corpus.len());
                } else {
                    panic!("random_select failed on non-empty corpus");
                }
            }
            assert_eq!(selected_ids_counts.len(), 3, "All items should be selected");
            for id_val in 0..3 {
                assert!(
                    selected_ids_counts.get(&id_val).unwrap_or(&0) > &0,
                    "ID {} not selected",
                    id_val
                );
            }
        }

        #[test]
        fn in_memory_corpus_load_initial_seeds_vec_u8() -> Result<(), CorpusError> {
            let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
            let temp_dir = tempdir().unwrap();
            let seed1_p = temp_dir.path().join("s1.bin");
            let seed2_p = temp_dir.path().join("s2.txt");
            fs::write(&seed1_p, [1, 2]).unwrap();
            fs::write(&seed2_p, [3, 4, 5]).unwrap();
            let seed_d = temp_dir.path().join("s_dir");
            fs::create_dir(&seed_d).unwrap();
            let seed3_p_d = seed_d.join("s3.dat");
            fs::write(&seed3_p_d, [6]).unwrap();
            let paths = vec![seed1_p.clone(), seed_d.clone(), seed2_p.clone()];
            let count = corpus.load_initial_seeds(&paths)?;
            assert_eq!(count, 3);
            assert_eq!(corpus.len(), 3);
            temp_dir.close().unwrap();
            Ok(())
        }
    }

    #[cfg(test)]
    mod on_disk_corpus_tests {
        use super::*;

        #[test]
        fn on_disk_corpus_new_dir_creation_and_empty_load() -> Result<(), CorpusError> {
            let base_dir = tempdir().unwrap();
            let corpus_p = base_dir.path().join("new_disk_corpus");
            assert!(!corpus_p.exists());
            let c: OnDiskCorpus<Vec<u8>> = OnDiskCorpus::new(corpus_p.clone())?;
            assert!(corpus_p.exists() && corpus_p.is_dir());
            assert_eq!(c.len(), 0);
            assert!(c.index_file_path.exists());
            let re_c: OnDiskCorpus<Vec<u8>> = OnDiskCorpus::new(corpus_p.clone())?;
            assert_eq!(re_c.len(), 0);
            base_dir.close().unwrap();
            Ok(())
        }

        #[test]
        fn on_disk_corpus_add_get_len_persistence_vec_u8() -> Result<(), CorpusError> {
            let dir = tempdir().unwrap();
            let c_path = dir.path().to_path_buf();
            let d1: Vec<u8> = vec![1, 2, 3];
            let m1_str = "meta1".to_string();
            {
                let mut c: OnDiskCorpus<Vec<u8>> = OnDiskCorpus::new(c_path.clone())?;
                let id1 = c.add(
                    d1.clone(),
                    Box::new(OnDiskCorpusEntryMetadata {
                        source_description: m1_str.clone(),
                    }),
                )?;
                assert_eq!(id1, 0);
                assert_eq!(c.len(), 1);
                let f_path = c.get_input_file_path_from_stem(&c.id_to_filename_stem[0]);
                assert!(f_path.exists());
                if let Some((ri, rmd)) = c.get(id1) {
                    assert_eq!(*ri, d1);
                    assert_eq!(
                        rmd.downcast_ref::<OnDiskCorpusEntryMetadata>()
                            .unwrap()
                            .source_description,
                        m1_str
                    );
                } else {
                    panic!("get failed");
                }
            }
            {
                let mut rc: OnDiskCorpus<Vec<u8>> = OnDiskCorpus::new(c_path)?;
                assert_eq!(rc.len(), 1);
                if let Some((ri, rmd)) = rc.get(0) {
                    assert_eq!(*ri, d1);
                    assert_eq!(
                        rmd.downcast_ref::<OnDiskCorpusEntryMetadata>()
                            .unwrap()
                            .source_description,
                        m1_str
                    );
                } else {
                    panic!("get from reloaded failed");
                }
            }
            dir.close().unwrap();
            Ok(())
        }

        #[test]
        fn on_disk_corpus_random_select_and_cache() -> Result<(), CorpusError> {
            let dir = tempdir().unwrap();
            let mut corpus: OnDiskCorpus<TestInputStruct> =
                OnDiskCorpus::new(dir.path().to_path_buf())?;
            let mut rng = ChaCha8Rng::from_seed([0; 32]);
            assert!(corpus.random_select(&mut rng).is_none());

            let input1 = TestInputStruct {
                data_bytes: vec![1],
                id_number: 101,
            };
            let input2 = TestInputStruct {
                data_bytes: vec![2, 2],
                id_number: 102,
            };
            corpus.add(input1.clone(), Box::new("m1_s".to_string()))?;
            corpus.add(
                input2.clone(),
                Box::new(OnDiskCorpusEntryMetadata {
                    source_description: "m2_st".to_string(),
                }),
            )?;
            assert_eq!(corpus.len(), 2);

            let result_select = corpus.random_select(&mut rng);
            assert!(result_select.is_some(), "First random_select failed");

            let (selected_id, selected_input, _) = result_select.unwrap();
            let selected_input_clone = selected_input.clone();

            {
                assert!(
                    corpus.last_accessed_cache.is_some(),
                    "Cache should be populated after random_select"
                );

                if let Some(ref cache) = corpus.last_accessed_cache {
                    assert_eq!(
                        cache.id, selected_id,
                        "Cache ID mismatch after random_select"
                    );
                    assert_eq!(
                        cache.input, selected_input_clone,
                        "Cache input mismatch after random_select"
                    );
                }
            }

            let result_get_same = corpus.get(selected_id);
            assert!(result_get_same.is_some(), "get for selected_id failed");

            let (get_input, _) = result_get_same.unwrap();
            assert_eq!(
                *get_input, selected_input_clone,
                "Input from get (cached) should match random_select"
            );

            {
                if let Some(ref cache) = corpus.last_accessed_cache {
                    assert_eq!(
                        cache.id, selected_id,
                        "Cache ID should still be selected_id"
                    );
                }
            }

            let other_id = if selected_id == 0 { 1 } else { 0 };
            let expected_other_input = if other_id == 0 {
                input1.clone()
            } else {
                input2.clone()
            };

            let result_get_other = corpus.get(other_id);
            assert!(result_get_other.is_some(), "get for other_id failed");

            let (get_other_input, _) = result_get_other.unwrap();
            assert_eq!(
                *get_other_input, expected_other_input,
                "Input from get (other_id) mismatch"
            );

            {
                assert!(
                    corpus.last_accessed_cache.is_some(),
                    "Cache should be populated after get(other_id)"
                );

                if let Some(ref cache) = corpus.last_accessed_cache {
                    assert_eq!(cache.id, other_id, "Cache ID should be updated to other_id");
                    assert_eq!(
                        cache.input, expected_other_input,
                        "Cache input mismatch for other_id"
                    );
                }
            }
            Ok(())
        }

        #[test]
        fn on_disk_corpus_load_initial_seeds_custom_struct() -> Result<(), CorpusError> {
            let t_dir = tempdir().unwrap();
            let s_path = t_dir.path().join("s_custom");
            fs::create_dir_all(&s_path).unwrap();
            let s1 = TestInputStruct {
                data_bytes: vec![1],
                id_number: 1,
            };
            let s1_f = s_path.join("s1.fuzzinput");
            let b_cfg = OnDiskCorpus::<TestInputStruct>::current_bincode_config();
            fs::write(&s1_f, bincode::encode_to_vec(&s1, b_cfg).unwrap()).unwrap();
            let main_c_p = t_dir.path().join("main_c_custom");
            let mut c: OnDiskCorpus<TestInputStruct> = OnDiskCorpus::new(main_c_p)?;
            let paths_load = vec![s1_f.clone()];
            let count = c.load_initial_seeds(&paths_load)?;
            assert_eq!(count, 1);
            assert_eq!(c.len(), 1);
            if let Some((i, _)) = c.get(0) {
                assert_eq!(*i, s1);
            } else {
                panic!("S1 not loaded");
            }
            t_dir.close().unwrap();
            Ok(())
        }

        #[test]
        fn on_disk_corpus_handles_invalid_path_for_new() {
            let dir = tempdir().unwrap();
            let f_path = dir.path().join("file.txt");
            File::create(&f_path).unwrap();
            let res = OnDiskCorpus::<Vec<u8>>::new(f_path);
            assert!(res.is_err());
            if let Err(CorpusError::Io(msg)) = res {
                assert!(msg.contains("not a directory"));
            }
            dir.close().unwrap();
        }

        #[test]
        fn on_disk_corpus_skips_index_and_dotfiles_when_loading_seeds_from_dir()
        -> Result<(), CorpusError> {
            let t_dir = tempdir().unwrap();
            let s_dir = t_dir.path().join("s_dir_skip");
            fs::create_dir_all(&s_dir).unwrap();
            let vs_d = TestInputStruct {
                data_bytes: vec![1],
                id_number: 1,
            };
            let vs_f = s_dir.join("good_seed.fuzzinput");
            let b_cfg = OnDiskCorpus::<TestInputStruct>::current_bincode_config();
            fs::write(&vs_f, bincode::encode_to_vec(&vs_d, b_cfg).unwrap()).unwrap();
            fs::write(
                s_dir.join(OnDiskCorpus::<TestInputStruct>::INDEX_FILENAME),
                "{}",
            )
            .unwrap();
            fs::write(s_dir.join(".hid"), "data").unwrap();
            let mc_p = t_dir.path().join("mc_skip");
            let mut c: OnDiskCorpus<TestInputStruct> = OnDiskCorpus::new(mc_p)?;
            let paths = vec![s_dir];
            let count = c.load_initial_seeds(&paths)?;
            assert_eq!(count, 1);
            assert_eq!(c.len(), 1);
            if let Some((i, _)) = c.get(0) {
                assert_eq!(*i, vs_d);
            } else {
                panic!("Valid seed not found");
            }
            t_dir.close().unwrap();
            Ok(())
        }
    }
}

#[cfg(test)]
pub(crate) mod test_utils {
    use super::*;
    #[derive(Debug, Default)]
    pub struct DummyTestCorpus;
    impl<I: Input> Corpus<I> for DummyTestCorpus {
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
