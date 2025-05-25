# Drifter Design Document

## 1. Introduction

Drifter is an extensible fuzzing framework written in Rust. It provides a foundation for fuzzing diverse targets, including libraries (via in-process execution) and external command-line applications. The framework is designed with modularity, allowing users to select and configure various components like mutators, executors, and corpus managers.

**Current Status:**
The project has implemented a core fuzzing loop capable of orchestrating different components. Key features include:
*   A command-line interface (`drifter_cli`) to run fuzzing campaigns.
*   A core library (`drifter_core`) defining fuzzing traits and providing initial implementations.
*   Configuration via TOML files (`DrifterConfig`) with CLI overrides.
*   Support for both in-process fuzzing (with panic handling) and out-of-process command execution (handling exit codes, timeouts, and different input delivery mechanisms).
*   Basic and structure-aware mutators.
*   In-memory and persistent on-disk corpus management, including seed loading.
*   Feedback mechanisms based on unique inputs and mock coverage.
*   Crash detection oracles.
*   Integration with external projects for in-process fuzzing (e.g., `rohcstar`).

## 2. Core Principles

*   **Extensibility:** Modular, trait-based design for core components (Mutators, Executors, Schedulers, Oracles, Corpus, Feedback) allowing for easy replacement or addition of new strategies.
*   **Configurability:** Fuzzing campaigns are configured via TOML files, with options for overriding key parameters through a CLI.
*   **Target Agnostic (to a degree):** Supports fuzzing both Rust library functions (in-process) and arbitrary external binaries (command-line).
*   **Robustness:** Aims to handle target crashes and timeouts gracefully.
*   **Developer Experience:** Provides a CLI for managing fuzzing sessions and clear configuration options.
*   **Reproducibility:** Intends to support deterministic fuzzing runs (though full determinism depends on RNG seeding and target behavior).
*   **Comprehensive Testing:** Includes unit tests for core components and integration tests using example target scripts.

## 3. Architecture Overview

Drifter is structured into two main crates:

*   **`drifter_core`:** Contains the foundational traits, data structures, and default implementations for all core fuzzing components. This includes:
    *   Traits: `Input`, `Mutator`, `Executor`, `Observer`, `Feedback`, `Corpus`, `Scheduler`, `Oracle`.
    *   Implementations: `FlipSingleByteMutator`, `JsonStructureAwareMutator`, `InProcessExecutor`, `CommandExecutor`, `InMemoryCorpus`, `OnDiskCorpus`, `CrashOracle`, `RandomScheduler`, `UniqueInputFeedback`, `MockBitmapCoverageFeedback`, `NoOpObserver`, `MockCoverageObserver`.
    *   Configuration: `DrifterConfig` and related structs for parsing TOML configurations.
    *   Error Handling: Custom error types for different modules.

*   **`drifter_cli`:** Provides the command-line interface and orchestrates the main fuzzing loop.
    *   Parses CLI arguments and loads `DrifterConfig`.
    *   Instantiates and wires together components from `drifter_core` based on the configuration.
    *   Manages the primary fuzzing cycle: input selection, mutation, execution, feedback, and oracle checks.
    *   Includes specific harness integrations (e.g., for `rohcstar`).

The general workflow involves the `drifter_cli` reading a configuration, setting up the chosen components (mutator, executor, corpus, etc.), and then running a loop where inputs are selected, mutated, executed against the target, and the results are fed back to improve future input generation or identify crashes.

## 4. Configuration System (`drifter_core/src/config.rs`)

Drifter uses a TOML-based configuration system, managed by the `DrifterConfig` struct.

*   **Loading:** Configuration is loaded from a TOML file (default: `drifter_config.toml`, can be specified via CLI).
*   **Structure:**
    *   `[fuzzer]`: General fuzzer settings like `max_iterations` and `threads`.
    *   `[executor]`: Defines the `executor_type` (`InProcess` or `Command`).
        *   `[executor.command-settings]`: Specific to `CommandExecutor`, includes `command` (Vec of executable and args), `input_delivery` (`StdIn` or `File` with template), `timeout_ms`, and `working_dir`.
        *   `[executor.in-process-settings]`: Specific to `InProcessExecutor`, includes `harness_key` to select the target function.
    *   `[corpus]`: Defines `corpus_type` (`InMemory` or `OnDisk`).
        *   Includes `initial_seed_paths` (Vec of PathBufs).
        *   Includes `on_disk_path` for `OnDiskCorpus`.
*   **Defaults:** Sensible defaults are provided for most settings if they are not specified in the TOML file.
*   **CLI Overrides:** The `drifter_cli` allows overriding `max_iterations` and the target command for `CommandExecutor` via command-line arguments.

## 5. Core Fuzzing Components

### 5.1. Input (`drifter_core/src/input.rs`)
*   **Trait:** `Input` (Send + Sync + Debug + 'static) defines `as_bytes()`, `len()`, `is_empty()`.
*   **Implementation:** `Vec<u8>` provides a default implementation.

### 5.2. Executor (`drifter_core/src/executor.rs`)
*   **Trait:** `Executor<I: Input>` defines `execute_sync()`.
*   **`ExecutionStatus` Enum:** `Ok`, `Timeout`, `Crash(String)`, `ObserverError(String)`, `Other(String)`.
*   **`InProcessExecutor<F>`:**
    *   Executes a harness function `F: Fn(&[u8])` directly.
    *   Catches panics from the harness function and reports them as `ExecutionStatus::Crash`.
    *   Selected via `harness_key` in configuration (e.g., "rohcstar_decompressor", "default_dummy").
*   **`CommandExecutor`:**
    *   Executes an external command.
    *   **Configuration (`CommandExecutorConfig`):** `command_with_args`, `input_delivery_mode`, `execution_timeout`, `working_directory`.
    *   **Input Delivery (`InputDeliveryMode`):**
        *   `StdIn`: Passes input via stdin.
        *   `File(template_string)`: Writes input to a temporary file and passes the path via a command-line argument template (e.g., `"--input {}"`).
    *   Handles process timeouts by killing the child process.
    *   Interprets non-zero exit codes and termination signals (on Unix) as crashes.
    *   `ProcessOutput` struct captures stdout, stderr, exit code, and signal for observers.

### 5.3. Mutator (`drifter_core/src/mutator.rs`)
*   **Trait:** `Mutator<I: Input, R: Rng>` defines `mutate()`.
*   **`FlipSingleByteMutator`:** Randomly selects a byte and adds a small random value to it. Handles empty or `None` inputs by creating a default.
*   **`JsonStructureAwareMutator<S, RngType>`:**
    *   Designed for structured inputs `S` that are bincode-serialized `Vec<u8>`.
    *   Process: bincode bytes -> `S` -> `serde_json::Value` -> mutate JSON -> `S` -> bincode bytes.
    *   Mutates JSON values (objects, arrays, strings, numbers, booleans) recursively based on probabilities and max depth.
    *   Handles deserialization failures by falling back to default or pre-mutation states.

### 5.4. Corpus (`drifter_core/src/corpus.rs`)
*   **Trait:** `Corpus<I: Input>` defines `add()`, `get()`, `random_select()`, `len()`, `is_empty()`, `load_initial_seeds()`.
*   **`CorpusError` Enum:** `InputNotFound`, `CorpusIsEmpty`, `Io`, `Serialization`, `Deserialization`.
*   **`InMemoryCorpus<I: Input>`:**
    *   Stores inputs and `Box<dyn Any + Send + Sync>` metadata in a `Vec`.
    *   `load_initial_seeds` expects `I: From<Vec<u8>>` and reads raw bytes from seed files/directories.
*   **`OnDiskCorpus<I: Input + Serialize + DeserializeOwned + Encode + Decode<()>>`:**
    *   Provides persistent storage.
    *   Stores individual inputs as bincode-serialized files (default extension: `.fuzzinput`).
    *   Maintains a JSON index file (`corpus_index.json` by default) mapping IDs to filename stems and filename stems to `OnDiskCorpusEntryMetadata`.
    *   `OnDiskCorpusEntryMetadata`: Contains `source_description` (String).
    *   Supports caching for the last accessed input to reduce I/O.
    *   `load_initial_seeds` expects bincode-serialized inputs and skips hidden files or its own index file.

### 5.5. Observer (`drifter_core/src/observer.rs`)
*   **Trait:** `Observer` (Send + Sync) defines `name()`, `pre_exec()`, `post_exec()`, `reset()`, `serialize_data()`, `as_any()`, `as_any_mut()`.
*   **`NoOpObserver`:** A default observer that performs no actions.
*   **`MockCoverageObserver`:**
    *   Simulates coverage by computing the MD5 hash of the input provided in `post_exec`.
    *   `serialize_data()` returns the 16-byte MD5 hash as `Vec<u8>`.
    *   Used for testing feedback mechanisms like `MockBitmapCoverageFeedback`.

### 5.6. Feedback (`drifter_core/src/feedback.rs`)
*   **Trait:** `Feedback<I: Input>` defines `name()`, `init()`, `is_interesting()`, `add_to_corpus()`, `as_any()`, `as_any_mut()`.
*   **`FeedbackError` Enum:** `InitializationFailed`, `CorpusInteraction`, `ObserverData`.
*   **`MockBitmapCoverageFeedback`:**
    *   Considers an input interesting if the coverage data (expected MD5 hash) from a *target observer* (specified by name) is new.
    *   Maintains a `HashSet` of `global_coverage_hashes`.
*   **`UniqueInputFeedback`:**
    *   Considers an input interesting if its own MD5 hash is new.
    *   Maintains a `HashSet` of `known_input_hashes`.
    *   Does not rely on external observer data.

### 5.7. Scheduler (`drifter_core/src/scheduler.rs`)
*   **Trait:** `Scheduler<I: Input>` defines `next()` and `report_feedback()`.
*   **`SchedulerError` Enum:** `CorpusEmpty`, `CorpusInteractionError`.
*   **`RandomScheduler`:** Selects the next input ID randomly from the corpus. `report_feedback` is a no-op.

### 5.8. Oracle (`drifter_core/src/oracle.rs`)
*   **Trait:** `Oracle<I: Input>` defines `examine()`.
*   **`BugReport<I: Input>` Struct:** Contains `input`, `description`, `input_hash`, `severity`.
*   **`CrashOracle`:** Reports a `BugReport` if `ExecutionStatus` is `Crash`. Uses MD5 for `input_hash`.

## 6. Fuzzing Loop (`drifter_cli/src/main.rs`)

The `drifter_cli` orchestrates the main fuzzing loop:
1.  Parses CLI arguments and loads/merges `DrifterConfig`.
2.  Initializes RNG, Mutator, Oracle, Executor (InProcess or Command based on config), Corpus (InMemory or OnDisk), Scheduler, and Feedback mechanisms.
3.  Loads initial seeds into the corpus if specified. Adds a default "INIT" seed if corpus remains empty.
4.  Initializes the main feedback mechanism (e.g., pre-populates with hashes of initial corpus entries).
5.  Iterates for `max_iterations`:
    a.  Scheduler selects an input ID from the corpus.
    b.  The corresponding input is cloned.
    c.  Mutator generates a new input based on the selected one.
    d.  Observers are reset.
    e.  Executor runs the target with the mutated input, invoking observers.
    f.  Feedback mechanism checks if the new input is interesting based on observer data. If so, it's added to the corpus.
    g.  Oracle examines the execution status for bugs. If a bug is found, a report is generated and the crashing input might be added to the corpus (if not already by feedback).
    h.  Scheduler is notified of feedback/solution.
    i.  Progress is logged periodically.
6.  Prints a summary at the end of the campaign.

## 7. Error Handling

*   `drifter_core` uses `anyhow::Error` for general error propagation in component methods (like `Mutator::mutate`, `Observer::pre_exec/post_exec`).
*   Custom error enums like `CorpusError`, `FeedbackError`, `SchedulerError` are defined for more specific error contexts within those modules. These often use `thiserror`.
*   `drifter_cli` uses `anyhow::Error` for its main error type and handles errors from core components.

## 8. Testing Strategy

*   **Unit Tests:** Present in `drifter_core` for individual components (e.g., mutators, corpus implementations, feedback mechanisms, observers, oracles, schedulers, executors).
*   **Test Targets (`test_targets/`):** A collection of shell scripts used to test the `CommandExecutor`. These scripts simulate various behaviors:
    *   Normal execution (`test_target_ok.sh`)
    *   Crashing with specific exit codes (`test_target_exit_code_crash.sh`, `test_target_crash.sh`)
    *   Crashing with signals (`test_target_signal_crash.sh`)
    *   Timeouts (`test_target_timeout.sh`)
    *   Reading from files (`test_target_reads_file.sh`, `test_target_file_check.sh`)
    These allow for robust testing of the command executor's ability to handle different target outcomes and input delivery modes.

## 9. Development Practices

*   **Workspace:** The project is organized as a Cargo workspace with `drifter_cli` and `drifter_core`.
*   **Git Hooks (`.cargo-husky/`):**
    *   `pre-commit`: Runs `cargo fmt --check`.
    *   `commit-msg`: Enforces Conventional Commits format.
    *   `pre-push`: Runs `cargo test --all` and `cargo audit`.
*   **Code Formatting:** `rustfmt` is used.

## 10. Roadmap

*   **Coverage-Guided Fuzzing:**
    *   Implement a functional `CoverageObserver` (e.g., using SanitizerCoverage or other instrumentation).
    *   Develop `BitmapCoverageFeedback` to use real coverage data.
*   **Stateful Fuzzing Enhancements:** (as mentioned in README vision)
    *   `StateManager` for modeling protocol states.
    *   State-aware input generation and mutation.
*   **Advanced Mutators:**
    *   Structure-aware mutators beyond JSON (e.g., for specific binary formats if needed by targets like Rohcstar).
    *   Dictionary-based mutators.
    *   Splicing mutators (combining parts of different corpus inputs).
*   **Advanced Schedulers:**
    *   Schedulers that prioritize inputs based on coverage, execution time, or other feedback metrics.
*   **More Oracles:**
    *   Oracles for detecting memory errors (e.g., ASan logs).
    *   Oracles for detecting specific application-level errors or invariants.
*   **Fuzzer State Persistence & Resumption:** Allow fuzzing campaigns to be paused and resumed, including corpus, statistics, and potentially scheduler/feedback state.
*   **Parallel Fuzzing:** Utilize the `threads` configuration for parallel execution (would require components to be fully `Send + Sync` and careful state management).
*   **Improved `OnDiskCorpus`:**
    *   More sophisticated indexing or caching if performance with very large corpora becomes an issue.
    *   Potentially more structured metadata beyond a simple string.
*   **Enhanced CLI:** More options for controlling fuzzing parameters, reporting, and corpus management.
*   **Further Rohcstar Integration:** Deepen the co-development, potentially driving more specialized mutators or state management features in Drifter based on Rohcstar's needs.
