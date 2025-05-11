pub mod config;
pub mod corpus;
pub mod executor;
pub mod feedback;
pub mod input;
pub mod mutator;
pub mod observer;
pub mod oracle;
pub mod scheduler;

pub use config::DrifterConfig;
pub use corpus::{Corpus, CorpusError, InMemoryCorpus};
pub use executor::{ExecutionStatus, Executor, InProcessExecutor};
pub use feedback::{Feedback, FeedbackError, UniqueInputFeedback};
pub use input::Input;
pub use mutator::{FlipSingleByteMutator, Mutator};
pub use observer::{NoOpObserver, Observer};
pub use oracle::{BugReport, CrashOracle, Oracle};
pub use scheduler::{RandomScheduler, Scheduler, SchedulerError};

mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
