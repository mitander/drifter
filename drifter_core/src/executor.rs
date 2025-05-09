use crate::input::Input;
use crate::observer::Observer;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionStatus {
    Ok,
    Timeout,
    Crash(String),
    Other(String),
}

pub trait Executor<I: Input> {
    fn execute_sync(&mut self, input: &I, observers: &mut [&mut dyn Observer]) -> ExecutionStatus;
}

pub struct InProcessExecutor<F>
where
    F: Fn(&[u8]),
{
    harness_fn: F,
}

impl<F> InProcessExecutor<F>
where
    F: Fn(&[u8]),
{
    pub fn new(harness_fn: F) -> Self {
        Self { harness_fn }
    }
}

impl<I: Input, F> Executor<I> for InProcessExecutor<F>
where
    F: Fn(&[u8]) + Send + Sync,
{
    fn execute_sync(&mut self, input: &I, observers: &mut [&mut dyn Observer]) -> ExecutionStatus {
        for obs in observers.iter_mut() {
            if obs.pre_exec().is_err() {}
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            (self.harness_fn)(input.as_bytes());
        }));

        let status = match result {
            Ok(_) => ExecutionStatus::Ok,
            Err(panic_payload) => {
                let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic type".to_string()
                };
                ExecutionStatus::Crash(msg)
            }
        };

        for obs in observers.iter_mut() {
            if obs.post_exec(&status, None).is_err() {}
        }
        status
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::NoOpObserver;

    fn simple_harness(data: &[u8]) {
        let _ = data;
    }

    fn panicking_harness(data: &[u8]) {
        if data.get(0) == Some(&0xFF) {
            panic!("Boom!");
        }
    }

    #[test]
    fn in_process_executor_runs_harness() {
        let mut executor = InProcessExecutor::new(simple_harness);
        let input_data: Vec<u8> = vec![1, 2, 3];
        let mut observer_instance = NoOpObserver;
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer_instance];
        let status = executor.execute_sync(&input_data, &mut observers);
        assert_eq!(status, ExecutionStatus::Ok);
    }

    #[test]
    fn in_process_executor_catches_panic() {
        let mut executor = InProcessExecutor::new(panicking_harness);
        let crashing_input: Vec<u8> = vec![0xFF];
        let mut observer_instance = NoOpObserver;
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer_instance];
        let status = executor.execute_sync(&crashing_input, &mut observers);
        match status {
            ExecutionStatus::Crash(msg) => assert!(msg.contains("Boom!")),
            _ => panic!("Expected a crash, got {:?}", status),
        }
    }
}
