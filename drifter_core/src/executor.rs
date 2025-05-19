use crate::input::Input;
use crate::observer::Observer;
use std::any::Any;
use std::fs::File;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile;

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
                eprintln!("ERROR: {error_msg}");
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
                eprintln!("ERROR: {error_msg}");
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

pub enum InputDelivery {
    StdIn,
    File(String),
}

pub struct CommandExecutorConfig {
    pub command: Vec<String>,
    pub input_delivery: InputDelivery,
    pub timeout: Duration,
    pub working_dir: Option<PathBuf>,
    // pub envs: Option<Vec<(String, String)>>, // TODO: environment variables
}

pub struct CommandExecutor {
    config: CommandExecutorConfig,
    // TODO: persistent state for the executor instance
}

impl CommandExecutor {
    pub fn new(config: CommandExecutorConfig) -> Self {
        Self { config }
    }

    fn run_and_wait_with_timeout(
        &self,
        mut child: Child,
        timeout: Duration,
    ) -> Result<std::process::ExitStatus, ExecutionStatus> {
        let start_time = Instant::now();

        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => {
                    if start_time.elapsed() > timeout {
                        eprintln!("Target timed out, killing...");
                        if let Err(e) = child.kill() {
                            eprintln!("Failed to kill child process: {e}");
                            return Err(ExecutionStatus::Other(format!(
                                "Failed to kill timed-out process: {e}",
                            )));
                        }
                        return Err(ExecutionStatus::Timeout);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    eprintln!("Error waiting for child process: {e}");
                    return Err(ExecutionStatus::Other(format!(
                        "Error waiting for child: {e}",
                    )));
                }
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

impl<I: Input> Executor<I> for CommandExecutor {
    fn execute_sync(&mut self, input: &I, observers: &mut [&mut dyn Observer]) -> ExecutionStatus {
        for obs in observers.iter_mut() {
            if let Err(e) = obs.pre_exec() {
                let error_msg = format!("Observer '{}' pre_exec failed: {}", obs.name(), e);
                eprintln!("ERROR: {error_msg}");
                return ExecutionStatus::ObserverError(error_msg);
            }
        }

        let mut cmd = Command::new(&self.config.command[0]);
        if self.config.command.len() > 1 {
            cmd.args(&self.config.command[1..]);
        }

        if let Some(cwd) = &self.config.working_dir {
            cmd.current_dir(cwd);
        }

        let mut temp_file_handle: Option<tempfile::NamedTempFile> = None;

        match &self.config.input_delivery {
            InputDelivery::StdIn => {
                cmd.stdin(Stdio::piped());
            }
            InputDelivery::File(arg_template) => {
                let named_temp_file = match tempfile::NamedTempFile::new() {
                    Ok(f) => f,
                    Err(e) => {
                        return ExecutionStatus::Other(format!("Failed to create temp file: {e}",));
                    }
                };
                if let Err(e) = File::create(named_temp_file.path())
                    .and_then(|mut f| f.write_all(input.as_bytes()))
                {
                    return ExecutionStatus::Other(format!(
                        "Failed to write to temp file {:?}: {}",
                        named_temp_file.path(),
                        e
                    ));
                }

                let path_str = match named_temp_file.path().to_str() {
                    Some(s) => s.to_string(),
                    None => {
                        return ExecutionStatus::Other(
                            "Temp file path is not valid UTF-8".to_string(),
                        );
                    }
                };

                let final_arg = arg_template.replace("{}", &path_str);
                // Split final_arg if it contains spaces and is meant to be multiple args
                // For simplicity here, assume arg_template is like "--file={}" or "{}"
                // If it's like "some_cmd {} --other-opt", then this split is naive.
                // A more robust way would be to have the template produce a Vec<String>
                // or have separate fields for prefix_args, input_arg_template, suffix_args.
                // For now, just add it as one arg (or series of args if it contains spaces)
                for part in final_arg.split_whitespace() {
                    cmd.arg(part);
                }
                temp_file_handle = Some(named_temp_file);
            }
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child_process = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                let error_msg =
                    format!("Failed to spawn command '{:?}': {}", self.config.command, e);
                eprintln!("ERROR: {error_msg}");
                let mut status_on_fail = ExecutionStatus::Other(error_msg.clone());
                for obs in observers.iter_mut() {
                    if let Err(e_obs) = obs.post_exec(&status_on_fail, None::<&dyn Any>) {
                        eprintln!(
                            "ERROR during post_exec after spawn fail: Observer '{}' post_exec failed: {}",
                            obs.name(),
                            e_obs
                        );
                        // If post_exec fails, update the status to ObserverError if it wasn't already a more severe error
                        if !matches!(status_on_fail, ExecutionStatus::ObserverError(_)) {
                            status_on_fail = ExecutionStatus::ObserverError(format!(
                                "Observer '{}' post_exec failed after spawn error: {}",
                                obs.name(),
                                e_obs
                            ));
                        }
                    }
                }
                return status_on_fail;
            }
        };

        if let InputDelivery::StdIn = self.config.input_delivery {
            if let Some(mut child_stdin) = child_process.stdin.take() {
                if let Err(e) = child_stdin.write_all(input.as_bytes()) {
                    eprintln!("Error writing to child stdin: {e}. Killing child.");
                    let _ = child_process.kill();
                    let _ = child_process.wait();
                    return ExecutionStatus::Other(format!("Failed to write to stdin: {e}"));
                }
            } else {
                return ExecutionStatus::Other(
                    "Child stdin was not available after piping.".to_string(),
                );
            }
        }

        // Wait for the child process with timeout
        let exit_status_result = self.run_and_wait_with_timeout(child_process, self.config.timeout);

        // After run_and_wait_with_timeout, child_process has been consumed (waited on).
        // To get stdout/stderr, we'd need to capture them from the child object before it's fully waited on,
        // typically by spawning threads to read them or using non-blocking reads in the try_wait loop.
        // For simplicity in this first pass, let's assume stdout/stderr are not captured here
        // directly into ProcessOutput but could be read by specialized Observers if they interact
        // with child.stdout/stderr handles (which would need to be managed carefully).
        // A more robust solution uses `child.wait_with_output()` but that blocks and makes custom timeout harder.
        //
        // Let's refine `run_and_wait_with_timeout` or use a crate for process management that handles this better.
        // For now, we'll just focus on the exit status. stdout/stderr capture will be more complex.
        // We'll create a placeholder ProcessOutput for now.
        let mut process_output_data = ProcessOutput::default();

        let final_status = match exit_status_result {
            Ok(status) => {
                process_output_data.exit_code = status.code();
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    process_output_data.signal = status.signal();
                }

                if status.success() {
                    ExecutionStatus::Ok
                } else {
                    let desc = if let Some(code) = status.code() {
                        format!("Exited with code {code}")
                    } else if cfg!(unix) {
                        #[cfg(unix)]
                        {
                            use std::os::unix::process::ExitStatusExt;
                            if let Some(signal) = status.signal() {
                                format!("Terminated by signal {signal}")
                            } else {
                                "Exited abnormally".to_string()
                            }
                        }
                        #[cfg(not(unix))]
                        {
                            "Exited abnormally".to_string()
                        }
                    } else {
                        "Exited abnormally".to_string()
                    };
                    ExecutionStatus::Crash(desc)
                }
            }
            Err(exec_status_from_wait) => exec_status_from_wait,
        };

        drop(temp_file_handle);

        let mut overall_status = final_status;
        let mut post_exec_error_msg: Option<String> = None;

        for obs in observers.iter_mut() {
            if let Err(e) = obs.post_exec(&overall_status, Some(&process_output_data as &dyn Any)) {
                let error_msg = format!("Observer '{}' post_exec failed: {}", obs.name(), e);
                eprintln!("ERROR: {error_msg}");
                if post_exec_error_msg.is_none() {
                    post_exec_error_msg = Some(error_msg);
                }
            }
        }

        if !matches!(overall_status, ExecutionStatus::Crash(_))
            && !matches!(overall_status, ExecutionStatus::Timeout)
            && !matches!(overall_status, ExecutionStatus::ObserverError(_))
            && post_exec_error_msg.is_some()
        {
            overall_status = ExecutionStatus::ObserverError(post_exec_error_msg.unwrap());
        }

        overall_status
    }
}

#[cfg(test)]
mod in_process_executor_tests {
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
            _ => panic!("Expected a crash, got {status:?}"),
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
            _ => panic!("Expected ObserverError, got {status:?}"),
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
            _ => panic!("Expected ObserverError, got {status:?}"),
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
            _ => panic!("Expected Crash, got {status:?}"),
        }
    }
}

#[cfg(test)]
mod command_executor_tests {
    use super::*;
    use crate::observer::NoOpObserver;

    fn get_test_target_path(name: &str) -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir.join("../test_targets").join(name)
    }

    #[test]
    fn cmd_exec_successful_run_stdin() {
        let target_path = get_test_target_path("test_target_ok.sh");
        if !target_path.exists() {
            panic!("Test target missing: {target_path:?}");
        }

        let config = CommandExecutorConfig {
            command: vec![target_path.to_str().unwrap().to_string()],
            input_delivery: InputDelivery::StdIn,
            timeout: Duration::from_secs(1),
            working_dir: None,
        };
        let mut executor = CommandExecutor::new(config);
        let input_data: Vec<u8> = b"hello".to_vec();
        let mut observer = NoOpObserver;
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];

        let status = executor.execute_sync(&input_data, &mut observers);
        assert_eq!(status, ExecutionStatus::Ok);
    }

    #[test]
    fn cmd_exec_crash_detection() {
        let target_path = get_test_target_path("test_target_crash.sh");
        if !target_path.exists() {
            panic!("Test target missing: {target_path:?}");
        }

        let config = CommandExecutorConfig {
            command: vec![target_path.to_str().unwrap().to_string()],
            input_delivery: InputDelivery::StdIn,
            timeout: Duration::from_secs(1),
            working_dir: None,
        };
        let mut executor = CommandExecutor::new(config);
        let input_data: Vec<u8> = vec![];
        let mut observer = NoOpObserver;
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];

        let status = executor.execute_sync(&input_data, &mut observers);
        match status {
            ExecutionStatus::Crash(desc) => {
                // On Unix, exit 139 means killed by signal 11 (SIGSEGV)
                assert!(
                    desc.contains("code 139") || desc.contains("signal 11"),
                    "Unexpected crash desc: {desc}",
                );
            }
            _ => panic!("Expected Crash status, got {status:?}"),
        }
    }

    #[test]
    fn cmd_exec_timeout() {
        let target_path = get_test_target_path("test_target_timeout.sh");
        if !target_path.exists() {
            panic!("Test target missing: {target_path:?}");
        }

        let config = CommandExecutorConfig {
            command: vec![target_path.to_str().unwrap().to_string()],
            input_delivery: InputDelivery::StdIn,
            timeout: Duration::from_millis(100), // test: 100ms, script: 5s
            working_dir: None,
        };
        let mut executor = CommandExecutor::new(config);
        let input_data: Vec<u8> = vec![];
        let mut observer = NoOpObserver;
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];

        let status = executor.execute_sync(&input_data, &mut observers);
        assert_eq!(status, ExecutionStatus::Timeout);
    }

    #[test]
    fn cmd_exec_input_via_file() {
        let target_path = get_test_target_path("test_target_file_check.sh");
        if !target_path.exists() {
            panic!("Test target missing: {target_path:?}");
        }

        let config = CommandExecutorConfig {
            command: vec![target_path.to_str().unwrap().to_string()],
            input_delivery: InputDelivery::File("{}".to_string()),
            timeout: Duration::from_secs(1),
            working_dir: None,
        };
        let mut executor = CommandExecutor::new(config);

        let input_ok: Vec<u8> = b"OK_FILE".to_vec();
        let input_crash: Vec<u8> = b"CRASHFILE".to_vec();
        let mut observer = NoOpObserver;
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];

        let status_ok = executor.execute_sync(&input_ok, &mut observers);
        assert_eq!(
            status_ok,
            ExecutionStatus::Ok,
            "Expected Ok for non-crashing file input"
        );

        let status_crash = executor.execute_sync(&input_crash, &mut observers);
        match status_crash {
            ExecutionStatus::Crash(desc) => {
                assert!(
                    desc.contains("code 1"),
                    "Expected crash with code 1, got: {desc}",
                );
            }
            _ => panic!("Expected Crash status for CRASHFILE, got {status_crash:?}",),
        }
    }

    #[test]
    fn cmd_exec_invalid_command() {
        let config = CommandExecutorConfig {
            command: vec!["./this_command_does_not_exist_ever_12345.sh".to_string()],
            input_delivery: InputDelivery::StdIn,
            timeout: Duration::from_secs(1),
            working_dir: None,
        };
        let mut executor = CommandExecutor::new(config);
        let input_data: Vec<u8> = vec![];
        let mut observer = NoOpObserver;
        let mut observers: Vec<&mut dyn Observer> = vec![&mut observer];

        let status = executor.execute_sync(&input_data, &mut observers);
        match status {
            ExecutionStatus::Other(msg) => {
                assert!(
                    msg.contains("Failed to spawn command")
                        || msg.contains("No such file or directory")
                );
            }
            _ => panic!("Expected Other status for invalid command, got {status:?}",),
        }
    }
}
