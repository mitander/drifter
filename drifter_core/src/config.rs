use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Specifies how fuzzing input is delivered to a command-line target executor.
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigInputDelivery {
    /// Input is passed to the target program via its standard input (stdin).
    /// This is the default delivery method if not specified.
    /// Example in TOML: `input-delivery = "StdIn"` (or omit for default).
    #[default]
    StdIn,
    /// Input is written to a temporary file, and the path to this file is passed
    /// as a command-line argument to the target program.
    /// Example in TOML: `input-delivery = { file = { template = "--input-file={}" } }`.
    File {
        /// A template string for the command-line argument that specifies the input file.
        /// The exact path to the temporary input file will replace the `{}` placeholder.
        /// For example, if `template` is `"-f {}"`, and the temp file is `/tmp/input123`,
        /// the argument passed to the target will be `"-f /tmp/input123"`.
        template: String,
    },
}

/// Configuration settings specific to the `CommandExecutor`.
///
/// These settings are used when `ExecutorConfig.executor_type` is set to `Command`.
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct CommandExecutorSettings {
    /// The command and its arguments to execute the target program.
    /// The first element of the vector is the program executable, and subsequent
    /// elements are its command-line arguments.
    /// Example: `command = ["./my_app", "--process-input", "--verbose"]`
    pub command: Vec<String>,
    /// Specifies the method for delivering the fuzz input to the target command.
    /// Defaults to `StdIn` if not specified.
    #[serde(default)]
    pub input_delivery: ConfigInputDelivery,
    /// Timeout for a single execution of the target command, in milliseconds.
    /// If the target runs longer than this, it's considered a timeout.
    /// Defaults to `2000` (2 seconds) if not specified.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// An optional path to the working directory from which the target command should be executed.
    /// If `None` (default), the command inherits the fuzzer's working directory.
    pub working_dir: Option<PathBuf>,
}

/// Returns the default timeout value (2000 milliseconds) for command execution.
/// Used by `serde` for `CommandExecutorSettings::timeout_ms`.
fn default_timeout_ms() -> u64 {
    2000
}

/// Defines the type of `Executor` to be used for running the target harness or program.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutorType {
    /// Executes the target harness function directly within the fuzzer's process.
    /// This is generally faster but requires the harness to be robust (e.g., panic-safe)
    /// as a panic can take down the fuzzer.
    InProcess,
    /// Executes the target as a separate command-line program.
    /// This is the default executor type. It's slower due to process creation overhead
    /// but more isolated, meaning a target crash doesn't directly crash the fuzzer.
    #[default]
    Command,
}

/// Configuration settings specific to the `InProcessExecutor`.
///
/// These settings are used when `ExecutorConfig.executor_type` is set to `InProcess`.
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct InProcessExecutorSettings {
    /// A string key used to identify which registered in-process harness function to execute.
    /// The fuzzer's main application must register harness functions with corresponding keys.
    /// If left empty (default), the behavior might depend on the fuzzer CLI allowing a single,
    /// unnamed default harness or requiring a key.
    #[serde(default)]
    pub harness_key: String,
}

/// Configuration for the `Executor` component of the fuzzer.
///
/// This struct does not derive `Default` itself; its default state is constructed
/// within `DrifterConfig::default()`.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ExecutorConfig {
    /// The type of executor to use (e.g., `InProcess` or `Command`).
    /// Defaults to `Command` if not specified.
    #[serde(default)]
    pub executor_type: ExecutorType,
    /// Settings for the `CommandExecutor`. This field is optional in the TOML.
    /// If `executor_type` is `Command`, these settings should typically be provided.
    /// If this field is entirely missing from TOML, it defaults to `None`.
    #[serde(default)]
    pub command_settings: Option<CommandExecutorSettings>,
    /// Settings for the `InProcessExecutor`. This field is optional in the TOML.
    /// If `executor_type` is `InProcess`, these settings should typically be provided.
    /// If this field is entirely missing from TOML, it defaults to `None`.
    #[serde(default)]
    pub in_process_settings: Option<InProcessExecutorSettings>,
}

/// Defines the type of `Corpus` storage to be used.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CorpusType {
    /// Corpus is stored entirely in the fuzzer's memory.
    /// This is the default type. It's generally faster for access but is not
    /// persistent across fuzzer runs.
    #[default]
    InMemory,
    /// Corpus is stored on disk in a specified directory.
    /// This allows for persistence across runs and can support larger corpora than
    /// available memory, but incurs I/O overhead.
    OnDisk,
}

impl CorpusType {
    /// Returns a string representation of the corpus type (e.g., "InMemory", "OnDisk").
    /// Useful for logging or display.
    pub fn to_string(&self) -> &str {
        match self {
            CorpusType::InMemory => "InMemory",
            CorpusType::OnDisk => "OnDisk",
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct CorpusConfig {
    /// The type of corpus storage to use (e.g., `InMemory` or `OnDisk`).
    /// Defaults to `InMemory` if not specified.
    #[serde(default = "default_corpus_type")]
    pub corpus_type: CorpusType,
    /// An optional list of paths to initial seed files or directories.
    #[serde(default)]
    pub initial_seed_paths: Option<Vec<PathBuf>>,
    /// The file system path to the directory for an on-disk corpus.
    #[serde(default = "default_on_disk_path")]
    pub on_disk_path: PathBuf,
}

impl Default for CorpusConfig {
    fn default() -> Self {
        Self {
            corpus_type: default_corpus_type(),
            initial_seed_paths: None,
            on_disk_path: default_on_disk_path(),
        }
    }
}

/// Returns the default file system path (`./.drifter_corpus`) for the on-disk corpus.
/// Used by `serde` for `CorpusConfig::on_disk_path`.
pub fn default_on_disk_path() -> PathBuf {
    PathBuf::from("./.drifter_corpus")
}

/// Returns the default `CorpusType` (`InMemory`).
/// Used by `serde` for `CorpusConfig::corpus_type`.
fn default_corpus_type() -> CorpusType {
    CorpusType::InMemory
}

/// General settings for the fuzzer's operation.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct FuzzerSettings {
    /// The maximum number of fuzzing iterations to perform before stopping.
    /// Defaults to `1,000,000` if not specified.
    #[serde(default = "default_iterations")]
    pub max_iterations: u64,
    /// The number of parallel threads to use for fuzzing.
    /// This setting might be for future use or specific fuzzer modes.
    /// Defaults to `1` if not specified.
    #[serde(default = "default_threads")]
    pub threads: usize,
}

/// Returns the default maximum number of fuzzing iterations (1,000,000).
/// Used by `serde` for `FuzzerSettings::max_iterations`.
pub fn default_iterations() -> u64 {
    1_000_000
}

/// Returns the default number of fuzzing threads (1).
/// Used by `serde` for `FuzzerSettings::threads`.
pub fn default_threads() -> usize {
    1
}

impl Default for FuzzerSettings {
    /// Provides default `FuzzerSettings`.
    fn default() -> Self {
        Self {
            max_iterations: default_iterations(),
            threads: default_threads(),
        }
    }
}

/// The main, top-level configuration structure for the Drifter fuzzer.
///
/// This struct is expected to be deserialized from the root of the TOML configuration file.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct DrifterConfig {
    /// Optional general fuzzer settings.
    /// If the `[fuzzer]` table is present in TOML (even if empty), `FuzzerSettings::default()` is used.
    /// If the `[fuzzer]` table is entirely missing, this field will be `None` after TOML parsing.
    /// However, `DrifterConfig::default()` initializes this to `Some(FuzzerSettings::default())`.
    #[serde(default)]
    pub fuzzer: Option<FuzzerSettings>,
    /// Configuration for the executor. This section (e.g., `[executor]`) is typically
    /// expected in the TOML file, as `ExecutorConfig` is not optional here.
    pub executor: ExecutorConfig,
    /// Optional corpus configuration.
    /// Similar to `fuzzer`, if `[corpus]` table is present, `CorpusConfig::default()` is used.
    /// If missing, this field is `None` after TOML parsing.
    /// `DrifterConfig::default()` initializes this to `Some(CorpusConfig::default())`.
    #[serde(default)]
    pub corpus: Option<CorpusConfig>,
}

impl DrifterConfig {
    /// Loads Drifter configuration by reading and parsing a TOML file from the given path.
    ///
    /// # Arguments
    /// * `path`: A reference to the file system path of the TOML configuration file.
    ///
    /// # Errors
    /// Returns `anyhow::Error` if the file cannot be read or if parsing the TOML content fails.
    pub fn load_from_file(path: &Path) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file at {:?}: {}", path, e))?;

        let config: DrifterConfig = toml::from_str(&content).map_err(|e| {
            anyhow::anyhow!("Failed to parse TOML from config file {:?}: {}", path, e)
        })?;

        Ok(config)
    }
}

impl Default for DrifterConfig {
    /// Provides a default `DrifterConfig` instance.
    ///
    /// This default is suitable for basic programmatic setup or as a fallback.
    /// A typical fuzzing run will load configuration from a TOML file.
    ///
    /// Default settings include:
    /// - Fuzzer: Uses `FuzzerSettings::default()`.
    /// - Executor: `Command` type. `command_settings` is `None` (requires user TOML config for actual command).
    ///   `in_process_settings` is `Some` with a default (empty) harness key.
    /// - Corpus: Uses `CorpusConfig::default()` (InMemory, no initial seeds, default on-disk path).
    fn default() -> Self {
        Self {
            fuzzer: Some(FuzzerSettings::default()),
            executor: ExecutorConfig {
                executor_type: ExecutorType::default(),
                command_settings: None,
                in_process_settings: Some(InProcessExecutorSettings::default()),
            },
            corpus: Some(CorpusConfig::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drifter_config_default_values_are_sensible() {
        let config = DrifterConfig::default();

        let fuzzer_settings = config
            .fuzzer
            .expect("Default DrifterConfig.fuzzer should be Some");
        assert_eq!(
            fuzzer_settings.max_iterations,
            default_iterations(),
            "Default max_iterations mismatch"
        );
        assert_eq!(
            fuzzer_settings.threads,
            default_threads(),
            "Default threads mismatch"
        );

        assert_eq!(
            config.executor.executor_type,
            ExecutorType::Command,
            "Default executor_type should be Command"
        );
        assert!(
            config.executor.command_settings.is_none(),
            "Default command_settings should be None"
        );
        let in_process_settings = config
            .executor
            .in_process_settings
            .expect("Default in_process_settings should be Some");
        assert_eq!(
            in_process_settings.harness_key, "",
            "Default harness_key should be empty"
        );

        let corpus_config = config
            .corpus
            .expect("Default DrifterConfig.corpus should be Some");
        assert_eq!(
            corpus_config.corpus_type,
            default_corpus_type(),
            "Default corpus_type should be InMemory"
        );
        assert!(
            corpus_config.initial_seed_paths.is_none(),
            "Default initial_seed_paths should be None"
        );
        assert_eq!(
            corpus_config.on_disk_path,
            default_on_disk_path(),
            "Default on_disk_path mismatch"
        );
    }

    #[test]
    fn load_minimal_config_from_toml_string() {
        let toml_content = r#"
        [executor]
        [executor.command-settings]
        command = ["./my_target_app", "arg1"]
        "#;
        let config: DrifterConfig =
            toml::from_str(toml_content).expect("Should parse minimal TOML content successfully");

        assert!(
            config.fuzzer.is_none(),
            "If [fuzzer] table is missing in TOML, DrifterConfig.fuzzer field (Option with #[serde(default)]) should be None"
        );

        assert_eq!(
            config.executor.executor_type,
            ExecutorType::Command,
            "Executor type should default to Command or be as specified"
        );
        let cmd_settings = config
            .executor
            .command_settings
            .expect("Command settings should be present as specified in TOML");
        assert_eq!(
            cmd_settings.command,
            vec!["./my_target_app".to_string(), "arg1".to_string()],
            "Command mismatch"
        );
        assert_eq!(
            cmd_settings.timeout_ms,
            default_timeout_ms(),
            "Timeout should default correctly"
        );
        match cmd_settings.input_delivery {
            ConfigInputDelivery::StdIn => {}
            _ => panic!("Input delivery should default to StdIn"),
        }
        assert!(
            config.executor.in_process_settings.is_none(),
            "In-process settings should be None as not specified"
        );

        assert!(
            config.corpus.is_none(),
            "If [corpus] table is missing in TOML, DrifterConfig.corpus field (Option with #[serde(default)]) should be None"
        );
    }

    #[test]
    fn load_config_with_all_optional_sections_present_and_defaulted() {
        let toml_content = r#"
        [fuzzer]
        [executor]
        executor-type = "in-process"
        [executor.in-process-settings]
        harness-key = "my_specific_harness"
        [corpus]
        "#;
        let config: DrifterConfig = toml::from_str(toml_content)
            .unwrap_or_else(|e| panic!("Should parse TOML with empty optional sections: {:?}", e));
        config.fuzzer.expect("[fuzzer] table was present");
        assert_eq!(config.executor.executor_type, ExecutorType::InProcess);
        config.corpus.expect("[corpus] table was present");
    }
    #[test]
    fn load_config_with_enum_file_delivery_correctly() {
        let toml_content = r#"
        [executor]
        # executor-type defaults to Command implicitly if not specified.
        [executor.command-settings]
        command = ["./target_bin"]
        input-delivery = { file = { template = "--use-file {}" } }
        "#;

        let config: DrifterConfig =
            toml::from_str(toml_content).expect("Should parse TOML with 'file' input delivery");

        let cmd_settings = config
            .executor
            .command_settings
            .expect("Command settings should be present");
        match cmd_settings.input_delivery {
            ConfigInputDelivery::File { template } => {
                assert_eq!(
                    template, "--use-file {}",
                    "Template string for file delivery mismatch"
                );
            }
            _ => panic!(
                "Expected ConfigInputDelivery::File, got {:?}",
                cmd_settings.input_delivery
            ),
        }
    }
}
