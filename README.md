# Drifter ドリフター

[![LICENSE](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Drifter is an extensible fuzzing framework written in Rust. It's designed for high performance and reliability, with an initial focus on uncovering vulnerabilities in stateful network protocol implementations.**

> [!WARNING]
> This fuzzing framework is in an early development phase.
> The API is unstable, features are incomplete, and breaking changes should be expected.

## Vision & Philosophy

*   Provide a versatile foundation for fuzzing diverse targets (libraries, binaries, network protocols).
*   Excel at modeling protocol state machines and generating contextually valid message sequences for deep bug discovery.
*   Be a reliable, performant, and developer-friendly tool for security researchers and software developers.

## Core Features (Conceptual / In Development)

*   **Extensible Architecture:** Modular, trait-based design for core components (Mutators, Executors, Schedulers, Oracles, Corpus, Feedback).
*   **Stateful Fuzzing Focus:** Specialized components for managing protocol state (`StateManager`) and generating state-aware inputs. **Drifter is actively being developed with [Rohcstar](https://github.com/mitander/rohcstar) as a primary test target to drive and validate these capabilities against a real-world network protocol.**
*   **Performance:** Built in Rust, leveraging `async` for I/O-bound operations.
*   **Reproducibility:** Deterministic fuzzing runs (given a seed and configuration).
*   **Comprehensive Testing:** Includes unit, integration, snapshot, property-based, and deterministic simulation testing (DST) for core logic.

## Design

For an in-depth understanding of Drifter's architecture, components, and roadmap, please refer to the [DESIGN_DOCUMENT.md](docs/DESIGN_DOCUMENT.md).

## License

Drifter is licensed under the [MIT License](LICENSE).
