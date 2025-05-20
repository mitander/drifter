//! `drifter_core`: The Foundational Fuzzing Framework Library
//!
//! `drifter_core` provides the essential building blocks for constructing and running
//! fuzzing campaigns with the Drifter fuzzing framework. It defines a set of core traits,
//! concrete implementations for common fuzzing components, and data structures for managing
//! the fuzzing process.
//!
//! ## Core Concepts
//!
//! The library is organized around several key abstractions:
//!
//! *   **[`Input`]**: Represents a single piece of data fed to the target program.
//!     Typically `Vec<u8>`, but can be any type implementing the `Input` trait.
//! *   **[`Mutator`]**: Transforms existing inputs to generate new, potentially interesting test cases.
//!     Implementations range from simple byte flips ([`FlipSingleByteMutator`]) to complex,
//!     structure-aware strategies ([`JsonStructureAwareMutator`]).
//! *   **[`Executor`]**: Responsible for running the target program with a given input.
//!     Supports both in-process execution ([`InProcessExecutor`]) for speed and
//!     out-of-process command execution ([`CommandExecutor`]) for robustness.
//!     Reports an [`ExecutionStatus`].
//! *   **[`Observer`]**: Monitors the execution of the target and collects data, such as
//!     code coverage ([`MockCoverageObserver`]) or other runtime behaviors.
//! *   **[`Feedback`]**: Analyzes data from `Observer`s to determine if an input is
//!     "interesting" (e.g., discovered new coverage) and should be added to the corpus.
//!     Examples include [`MockBitmapCoverageFeedback`] and [`UniqueInputFeedback`].
//! *   **[`Corpus`]**: Stores and manages the collection of fuzzing inputs.
//!     Implementations include [`InMemoryCorpus`] and persistent [`OnDiskCorpus`].
//! *   **[`Scheduler`]**: Selects the next input from the corpus to be fuzzed.
//!     A basic [`RandomScheduler`] is provided.
//! *   **[`Oracle`]**: Examines the outcome of an execution (e.g., [`ExecutionStatus`])
//!     to detect bugs, such as crashes ([`CrashOracle`]). Produces a [`BugReport`].
//! *   **[`config::DrifterConfig`]**: A comprehensive structure for configuring all aspects
//!     of the fuzzer, typically loaded from a TOML file.
//!
//! ## How to Use
//!
//! Typically, `drifter_core` is used as a library dependency by a command-line fuzzer
//! application (like `drifter_cli`). The CLI application would:
//!
//! 1.  Parse a configuration file ([`DrifterConfig`]).
//! 2.  Instantiate the chosen fuzzer components (Executor, Mutator, Corpus, etc.) based
//!     on the configuration.
//! 3.  Initialize the `Feedback` mechanism(s) with the initial `Corpus` state.
//! 4.  Enter the main fuzzing loop:
//!     a.  The `Scheduler` selects an input from the `Corpus`.
//!     b.  The `Mutator` transforms this input.
//!     c.  The `Executor` runs the target with the mutated input, invoking `Observer`s.
//!     d.  `Observer`s provide data to the `Feedback` mechanism.
//!     e.  If `Feedback` deems the input interesting, it's added to the `Corpus`.
//!     f.  The `Oracle` checks for bugs.
//!     g.  Repeat.
//!
//! This crate provides the "engine" parts; the "driver" (the fuzzing loop orchestrator)
//! is typically built on top of it.
pub mod config;
pub mod corpus;
pub mod executor;
pub mod feedback;
pub mod input;
pub mod mutator;
pub mod observer;
pub mod oracle;
pub mod scheduler;

pub use corpus::Corpus;
pub use executor::Executor;
pub use feedback::Feedback;
pub use input::Input;
pub use mutator::Mutator;
pub use observer::Observer;
pub use oracle::Oracle;
pub use scheduler::Scheduler;

pub use config::DrifterConfig;
pub use corpus::{CorpusError, InMemoryCorpus, OnDiskCorpus, OnDiskCorpusEntryMetadata};
pub use executor::{
    CommandExecutor, CommandExecutorConfig, ExecutionStatus, InProcessExecutor, InputDeliveryMode,
    ProcessOutput,
};
pub use feedback::{FeedbackError, MockBitmapCoverageFeedback, UniqueInputFeedback};
pub use mutator::{FlipSingleByteMutator, JsonStructureAwareMutator};
pub use observer::{MockCoverageObserver, NoOpObserver};
pub use oracle::{BugReport, CrashOracle};
pub use scheduler::{RandomScheduler, SchedulerError};
