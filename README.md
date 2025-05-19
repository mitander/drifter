# Drifter ドリフター

[![LICENSE](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Drifter is an extensible fuzzing framework written in Rust. It's designed for high performance and reliability, with an initial focus on uncovering vulnerabilities in stateful network protocol implementations.**

## Vision & Philosophy

*   Provide a versatile foundation for fuzzing diverse targets (libraries, binaries, network protocols).
*   Excel at modeling protocol state machines and generating contextually valid message sequences for deep bug discovery.
*   Be a reliable, performant, and developer-friendly tool for security researchers and software developers.

## Core Features (Conceptual / In Development)

*   **Extensible Architecture:** Modular, trait-based design for core components (Mutators, Executors, Schedulers, Oracles, Corpus, Feedback).
*   **Stateful Fuzzing Focus:** Specialized components for managing protocol state (`StateManager`) and generating state-aware inputs.
*   **Performance:** Built in Rust, leveraging `async` for I/O-bound operations.
*   **Reproducibility:** Deterministic fuzzing runs (given a seed and configuration).
*   **Comprehensive Testing:** Includes unit, integration, snapshot, property-based, and deterministic simulation testing (DST) for core logic.

## Current Status

Drifter is currently in the early stages of development (Phase 1: Core Framework MVP). The foundational components are being implemented.

*   [x] **Core Fuzzing Traits Defined:** `Input`, `Mutator`, `Executor`, `Observer`, `Feedback`, `Corpus`, `Scheduler`, `Oracle`.
*   [x] **Basic Fuzzing Loop:** Implemented in `drifter_cli`, orchestrating component interactions.
*   [x] **Configuration System:** Supports loading from TOML (`DrifterConfig`) with CLI overrides for key parameters.
*   [x] **Implemented Core Components (Initial Versions):**
    *   **Input:** `Vec<u8>` implementation.
    *   **Mutator:** `FlipSingleByteMutator`.
    *   **Executor:**
        *   `InProcessExecutor` (for fuzzing library functions, handles panics).
        *   `CommandExecutor` (for external binaries, handles exit codes, timeouts, stdin/file input).
    *   **Oracle:** `CrashOracle` (detects crashes/panics).
    *   **Corpus:**
        *   `InMemoryCorpus`.
        *   `OnDiskCorpus` (basic functionality: writes inputs, loads/saves JSON index, simple caching).
    *   **Scheduler:** `RandomScheduler`.
    *   **Feedback:** `UniqueInputFeedback` (tracks unique input hashes to guide corpus addition).
    *   **Observer:** `NoOpObserver`.
*   [ ] **Coverage-Guided Fuzzing (Next Major Focus):**
    *   [ ] `CoverageObserver` (Initial mock for stable, real implementation for nightly).
    *   [ ] `BitmapCoverageFeedback`.
*   [ ] **Robust Fuzzer State Persistence & Resumption.**
*   [ ] **Advanced Mutators (e.g., structure-aware - driven by Rohcstar needs).**
*   [ ] **Stateful Fuzzing Components (e.g., `StateManager` - driven by Rohcstar needs).**

## Design

For an in-depth understanding of Drifter's architecture, components, and roadmap, please refer to the [DESIGN_DOCUMENT.md](docs/DESIGN_DOCUMENT.md).

## License

Drifter is licensed under the [MIT License](LICENSE).
