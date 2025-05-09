use crate::input::Input;
use crate::observer::Observer;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionStatus {
    Ok,
    Timeout,
    Crash(String),
    ObserverError(String),
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
            if let Err(e) = obs.pre_exec() {
                let error_msg = format!("Observer '{}' pre_exec failed: {}", obs.name(), e);
                eprintln!("ERROR: {}", error_msg);
                return ExecutionStatus::ObserverError(error_msg);
            }
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            (self.harness_fn)(input.as_bytes());
        }));

        let mut execution_status = match result {
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

        let mut post_exec_error: Option<String> = None;
        for obs in observers.iter_mut() {
            if let Err(e) = obs.post_exec(&execution_status, None::<&dyn Any>) {
                let error_msg = format!("Observer '{}' post_exec failed: {}", obs.name(), e);
                eprintln!("ERROR: {}", error_msg);
                if post_exec_error.is_none() {
                    post_exec_error = Some(error_msg);
                }
            }
        }

        if execution_status == ExecutionStatus::Ok && post_exec_error.is_some() {
            execution_status = ExecutionStatus::ObserverError(post_exec_error.unwrap());
        }
        execution_status
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::{NoOpObserver, Observer};
    use std::any::Any;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn simple_harness(data: &[u8]) {
        let _ = data;
    }

    fn panicking_harness(data: &[u8]) {
        if data.first() == Some(&0xFF) {
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

    struct FailingPreExecObserver {
        should_fail_pre: bool,
        fail_count: AtomicUsize,
    }
    impl Observer for FailingPreExecObserver {
        fn name(&self) -> &'static str {
            "FailingPreExecObserver"
        }
        fn pre_exec(&mut self) -> Result<(), anyhow::Error> {
            if self.should_fail_pre {
                self.fail_count.fetch_add(1, Ordering::SeqCst);
                Err(anyhow::anyhow!("pre_exec intentional failure"))
            } else {
                Ok(())
            }
        }
        fn post_exec(
            &mut self,
            _status: &ExecutionStatus,
            _target_output: Option<&dyn Any>,
        ) -> Result<(), anyhow::Error> {
            Ok(())
        }
        fn reset(&mut self) -> Result<(), anyhow::Error> {
            Ok(())
        }
        fn serialize_data(&self) -> Option<Vec<u8>> {
            None
        }
    }

    struct FailingPostExecObserver {
        should_fail_post: bool,
        fail_count: AtomicUsize,
    }
    impl Observer for FailingPostExecObserver {
        fn name(&self) -> &'static str {
            "FailingPostExecObserver"
        }
        fn pre_exec(&mut self) -> Result<(), anyhow::Error> {
            Ok(())
        }
        fn post_exec(
            &mut self,
            _status: &ExecutionStatus,
            _target_output: Option<&dyn Any>,
        ) -> Result<(), anyhow::Error> {
            if self.should_fail_post {
                self.fail_count.fetch_add(1, Ordering::SeqCst);
                Err(anyhow::anyhow!("post_exec intentional failure"))
            } else {
                Ok(())
            }
        }
        fn reset(&mut self) -> Result<(), anyhow::Error> {
            Ok(())
        }
        fn serialize_data(&self) -> Option<Vec<u8>> {
            None
        }
    }

    #[test]
    fn executor_handles_observer_pre_exec_failure() {
        let mut executor = InProcessExecutor::new(simple_harness);
        let input_data: Vec<u8> = vec![1];
        let mut failing_observer = FailingPreExecObserver {
            should_fail_pre: true,
            fail_count: AtomicUsize::new(0),
        };
        let mut observers: Vec<&mut dyn Observer> = vec![&mut failing_observer];

        let status = executor.execute_sync(&input_data, &mut observers);
        match status {
            ExecutionStatus::ObserverError(msg) => {
                assert!(msg.contains("pre_exec intentional failure"));
                assert_eq!(failing_observer.fail_count.load(Ordering::SeqCst), 1);
            }
            _ => panic!("Expected ObserverError, got {:?}", status),
        }
    }

    #[test]
    fn executor_handles_observer_post_exec_failure() {
        let mut executor = InProcessExecutor::new(simple_harness);
        let input_data: Vec<u8> = vec![1];
        let mut failing_observer = FailingPostExecObserver {
            should_fail_post: true,
            fail_count: AtomicUsize::new(0),
        };
        let mut observers: Vec<&mut dyn Observer> = vec![&mut failing_observer];

        let status = executor.execute_sync(&input_data, &mut observers);
        match status {
            ExecutionStatus::ObserverError(msg) => {
                assert!(msg.contains("post_exec intentional failure"));
                assert_eq!(failing_observer.fail_count.load(Ordering::SeqCst), 1);
            }
            _ => panic!("Expected ObserverError, got {:?}", status),
        }
    }

    #[test]
    fn executor_reports_crash_even_if_post_exec_fails() {
        let mut executor = InProcessExecutor::new(panicking_harness);
        let crashing_input: Vec<u8> = vec![0xFF];
        let mut failing_observer = FailingPostExecObserver {
            should_fail_post: true,
            fail_count: AtomicUsize::new(0),
        };
        let mut observers: Vec<&mut dyn Observer> = vec![&mut failing_observer];

        let status = executor.execute_sync(&crashing_input, &mut observers);
        match status {
            ExecutionStatus::Crash(msg) => {
                assert!(msg.contains("Boom!"));
                assert_eq!(failing_observer.fail_count.load(Ordering::SeqCst), 1);
            }
            _ => panic!("Expected Crash, got {:?}", status),
        }
    }
}
