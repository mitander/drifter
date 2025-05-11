use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigInputDelivery {
    StdIn,
    File { template: String },
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct CommandExecutorSettings {
    pub command: Vec<String>,
    pub input_delivery: ConfigInputDelivery,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    pub working_dir: Option<PathBuf>,
    // TODO: pub envs: Option<HashMap<String, String>>,
}

fn default_timeout_ms() -> u64 {
    2000
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutorType {
    InProcess,
    Command,
}

fn default_executor_type() -> ExecutorType {
    ExecutorType::Command
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ExecutorConfig {
    #[serde(default = "default_executor_type")]
    pub executor_type: ExecutorType,
    pub command_settings: Option<CommandExecutorSettings>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct CorpusConfig {
    #[serde(default = "default_corpus_type")]
    pub corpus_type: String,
    pub initial_seed_paths: Option<Vec<PathBuf>>,
    pub on_disk_path: Option<PathBuf>,
}

fn default_corpus_type() -> String {
    "InMemory".to_string()
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
    pub fuzzer: Option<FuzzerSettings>,
    pub executor: ExecutorConfig,
    pub corpus: Option<CorpusConfig>,
    // TODO: Add sections for Mutator, Scheduler, Feedback, Oracle config
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
                executor_type: default_executor_type(),
                command_settings: None,
            },
            corpus: Some(CorpusConfig {
                corpus_type: default_corpus_type(),
                initial_seed_paths: None,
                on_disk_path: None,
            }),
        }
    }
}
