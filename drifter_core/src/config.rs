use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigInputDelivery {
    #[default]
    StdIn,
    File {
        template: String,
    },
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct CommandExecutorSettings {
    pub command: Vec<String>,
    pub input_delivery: ConfigInputDelivery,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    pub working_dir: Option<PathBuf>,
}

fn default_timeout_ms() -> u64 {
    2000
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutorType {
    InProcess,
    #[default]
    Command,
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct InProcessExecutorSettings {
    #[serde(default)]
    pub harness_key: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ExecutorConfig {
    #[serde(default)]
    pub executor_type: ExecutorType,
    #[serde(default)]
    pub command_settings: Option<CommandExecutorSettings>,
    #[serde(default)]
    pub in_process_settings: Option<InProcessExecutorSettings>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CorpusType {
    #[default]
    InMemory,
    OnDisk,
}

impl CorpusType {
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
    #[serde(default = "default_corpus_type")]
    pub corpus_type: CorpusType,
    pub initial_seed_paths: Option<Vec<PathBuf>>,
    #[serde(default = "default_on_disk_path")]
    pub on_disk_path: PathBuf,
}

pub fn default_on_disk_path() -> PathBuf {
    PathBuf::from("./.drifter_corpus")
}

fn default_corpus_type() -> CorpusType {
    CorpusType::InMemory
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct FuzzerSettings {
    #[serde(default = "default_iterations")]
    pub max_iterations: u64,
    #[serde(default = "default_threads")]
    pub threads: usize,
}

pub fn default_iterations() -> u64 {
    1_000_000
}
pub fn default_threads() -> usize {
    1
}

impl Default for FuzzerSettings {
    fn default() -> Self {
        Self {
            max_iterations: default_iterations(),
            threads: default_threads(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct DrifterConfig {
    #[serde(default)]
    pub fuzzer: Option<FuzzerSettings>,
    pub executor: ExecutorConfig,
    #[serde(default)]
    pub corpus: Option<CorpusConfig>,
}

impl DrifterConfig {
    pub fn load_from_file(path: &PathBuf) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file at {:?}: {}", path, e))?;

        let config: DrifterConfig = toml::from_str(&content).map_err(|e| {
            anyhow::anyhow!("Failed to parse TOML from config file {:?}: {}", path, e)
        })?;

        Ok(config)
    }
}

impl Default for DrifterConfig {
    fn default() -> Self {
        Self {
            fuzzer: Some(FuzzerSettings::default()),
            executor: ExecutorConfig {
                executor_type: ExecutorType::Command,
                command_settings: None,
                in_process_settings: Some(InProcessExecutorSettings::default()),
            },
            corpus: Some(CorpusConfig {
                corpus_type: default_corpus_type(),
                initial_seed_paths: None,
                on_disk_path: default_on_disk_path(),
            }),
        }
    }
}
