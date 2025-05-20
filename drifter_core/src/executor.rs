use crate::input::Input;
use crate::observer::Observer;
use std::any::Any;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::{self, NamedTempFile};

/// Represents the status of a single execution of the target program or harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionStatus {
    /// The target executed successfully without any detected issues.
    Ok,
    /// The target execution exceeded the configured timeout.
    Timeout,
    /// The target crashed or panicked during execution.
    /// Contains a string describing the crash (e.g., panic message, signal).
    Crash(String),
    /// An error occurred within an `Observer` during its `pre_exec` or `post_exec` hooks.
    /// Contains a string describing the observer error.
    ObserverError(String),
    /// Any other execution error not covered by the above categories (e.g., I/O error,
    /// failure to spawn process). Contains a descriptive string.
    Other(String),
}

/// An `Executor` is responsible for running a target (either an in-process harness
/// or an external command) with a given `Input` and managing `Observer` interactions.
///
/// # Type Parameters
/// * `I`: The type of `Input` that this executor processes.
pub trait Executor<I: Input> {
    /// Executes the target synchronously with the provided `input`.
    ///
    /// This method handles the full lifecycle of a single execution:
    /// 1. Calls `pre_exec` on all observers.
    /// 2. Runs the target with the input.
    /// 3. Captures the execution status (Ok, Crash, Timeout, etc.).
    /// 4. Calls `post_exec` on all observers with the status and any target output.
    ///
    /// # Arguments
    /// * `input`: A reference to the `Input` to be fed to the target.
    /// * `observers`: A mutable slice of `Observer` trait objects that will monitor
    ///   this execution. They are mutable to allow internal state changes.
    ///
    /// # Returns
    /// The `ExecutionStatus` of the target's run.
    fn execute_sync(&mut self, input: &I, observers: &mut [&mut dyn Observer]) -> ExecutionStatus;
}

/// An `Executor` that runs a harness function directly within the fuzzer's process.
///
/// This is generally faster than `CommandExecutor` due to no process overhead, but it's less safe.
/// If the harness function panics, it will typically crash the entire fuzzer process unless
/// the panic is caught (as this executor attempts to do).
///
/// # Type Parameters
/// * `F`: The type of the harness function. It must be a closure or function pointer
///   that takes a `&[u8]` (the input bytes) and is `Send + Sync` to allow potential
///   multi-threaded fuzzing in the future (though the executor itself is synchronous).
pub struct InProcessExecutor<F>
where
    F: Fn(&[u8]) + Send + Sync,
{
    harness_fn: F,
}

impl<F> InProcessExecutor<F>
where
    F: Fn(&[u8]) + Send + Sync,
{
    /// Creates a new `InProcessExecutor` with the given harness function.
    ///
    /// # Arguments
    /// * `harness_fn`: The function to be executed with each input. It must take
    ///   a byte slice (`&[u8]`) as input.
    pub fn new(harness_fn: F) -> Self {
        Self { harness_fn }
    }
}

impl<I: Input, F> Executor<I> for InProcessExecutor<F>
where
    F: Fn(&[u8]) + Send + Sync,
{
    fn execute_sync(&mut self, input: &I, observers: &mut [&mut dyn Observer]) -> ExecutionStatus {
        // 1. Call pre_exec on all observers
        for obs in observers.iter_mut() {
            if let Err(e) = obs.pre_exec() {
                let error_msg = format!("Observer '{}' pre_exec failed: {}", obs.name(), e);
                eprintln!("ERROR: {}", error_msg);
                return ExecutionStatus::ObserverError(error_msg);
            }
        }

        // 2. Execute the harness function, catching panics
        // AssertUnwindSafe is used because we are calling C or other FFI code
        // (or Rust code that might panic) and want to catch the panic.
        let execution_result = catch_unwind(AssertUnwindSafe(|| {
            (self.harness_fn)(input.as_bytes());
        }));

        // Determine initial status based on panic or successful completion
        let mut primary_status = match execution_result {
            Ok(_) => ExecutionStatus::Ok,
            Err(panic_payload) => {
                // Try to extract a meaningful message from the panic payload
                let panic_message = if let Some(s_ref) = panic_payload.downcast_ref::<&str>() {
                    s_ref.to_string()
                } else if let Some(s_obj) = panic_payload.downcast_ref::<String>() {
                    s_obj.clone()
                } else {
                    "Unknown panic type".to_string()
                };
                ExecutionStatus::Crash(panic_message)
            }
        };

        // 3. Call post_exec on all observers
        let mut post_exec_observer_error: Option<String> = None;
        for obs in observers.iter_mut() {
            // Pass None for target_output as InProcessExecutor doesn't capture stdout/stderr directly.
            // Observers needing such data would typically instrument the harness or use other means.
            if let Err(e) =
                obs.post_exec(&primary_status, None::<&dyn Any>, Some(input as &dyn Input))
            {
                let error_msg = format!("Observer '{}' post_exec failed: {}", obs.name(), e);
                eprintln!("ERROR: {}", error_msg);
                if post_exec_observer_error.is_none() {
                    // Report the first observer error encountered during post_exec
                    post_exec_observer_error = Some(error_msg);
                }
            }
        }

        // 4. Finalize status: If execution was Ok but an observer failed in post_exec,
        //    then the overall status becomes ObserverError. Crash status takes precedence.
        if primary_status == ExecutionStatus::Ok {
            if let Some(err_msg) = post_exec_observer_error {
                primary_status = ExecutionStatus::ObserverError(err_msg);
            }
        } else if let ExecutionStatus::Crash(_) = primary_status {
            if let Some(err_msg) = post_exec_observer_error {
                eprintln!(
                    "INFO: Observer error during post_exec after crash: {}",
                    err_msg
                );
            }
        }
        primary_status
    }
}

/// Specifies how input is delivered to a target executed by `CommandExecutor`.
#[derive(Debug, Clone)]
pub enum InputDeliveryMode {
    /// Input is passed via standard input (stdin).
    StdIn,
    /// Input is written to a temporary file, and the filename is passed as an argument.
    /// The `String` contains the command-line argument template where `{}` is the placeholder
    /// for the temporary filename. Example: `"--file={}"`
    File(String),
}

/// Configuration for a `CommandExecutor` instance.
/// This is typically derived from `crate::config::CommandExecutorSettings`.
#[derive(Debug, Clone)]
pub struct CommandExecutorConfig {
    /// The command and its arguments. First element is the executable.
    pub command_with_args: Vec<String>,
    /// How input is delivered to the target command.
    pub input_delivery_mode: InputDeliveryMode,
    /// Execution timeout for the target command.
    pub execution_timeout: Duration,
    /// Optional working directory for the command.
    pub working_directory: Option<PathBuf>,
}

/// An `Executor` that runs the target as an external command-line program.
///
/// This provides better isolation than `InProcessExecutor`, as a crash in the target
/// program does not directly crash the fuzzer. However, it incurs overhead due to
/// process creation and inter-process communication (if any).
pub struct CommandExecutor {
    config: CommandExecutorConfig,
}

impl CommandExecutor {
    /// Creates a new `CommandExecutor` with the given runtime configuration.
    pub fn new(config: CommandExecutorConfig) -> Self {
        Self { config }
    }

    /// Runs the spawned child process and waits for it to complete or timeout.
    /// Handles killing the process if it times out.
    fn run_and_wait_with_timeout(
        &self,
        mut child: Child,
        timeout: Duration,
    ) -> Result<std::process::ExitStatus, ExecutionStatus> {
        let start_time = Instant::now();

        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status), // Process exited
                Ok(None) => {
                    if start_time.elapsed() >= timeout {
                        eprintln!(
                            "INFO: Target command timed out after {:?}. Killing process...",
                            timeout
                        );
                        if let Err(e) = child.kill() {
                            eprintln!("ERROR: Failed to kill timed-out child process: {}", e);
                            return Err(ExecutionStatus::Other(format!(
                                "Target timed out, and failed to kill process: {}",
                                e
                            )));
                        }
                        match child.wait() {
                            Ok(_status) => return Err(ExecutionStatus::Timeout),
                            Err(e) => {
                                eprintln!("ERROR: Error waiting for killed child process: {}", e);
                                return Err(ExecutionStatus::Other(format!(
                                    "Target timed out, killed, but error during final wait: {}",
                                    e
                                )));
                            }
                        }
                    }
                    // Minimal sleep to avoid busy-waiting excessively
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    eprintln!("ERROR: Error attempting to wait for child process: {}", e);
                    return Err(ExecutionStatus::Other(format!(
                        "Failed to wait for child process: {}",
                        e
                    )));
                }
            }
        }
    }
}

/// Represents the collected output from an external process execution.
/// This can be passed to observers via `post_exec`'s `target_output` argument.
#[derive(Debug, Default, Clone)]
pub struct ProcessOutput {
    /// Content of the target's standard output (stdout).
    pub stdout_bytes: Vec<u8>,
    /// Content of the target's standard error (stderr).
    pub stderr_bytes: Vec<u8>,
    /// The exit code of the process, if it exited normally.
    pub exit_code: Option<i32>,
    /// The signal number that terminated the process, if it was killed by a signal (Unix-specific).
    pub signal_code: Option<i32>,
}

impl<I: Input> Executor<I> for CommandExecutor {
    fn execute_sync(&mut self, input: &I, observers: &mut [&mut dyn Observer]) -> ExecutionStatus {
        // 1. Call pre_exec on all observers
        for obs in observers.iter_mut() {
            if let Err(e) = obs.pre_exec() {
                let error_msg = format!("Observer '{}' pre_exec failed: {}", obs.name(), e);
                eprintln!("ERROR: {}", error_msg);
                return ExecutionStatus::ObserverError(error_msg);
            }
        }

        // 2. Prepare the command
        if self.config.command_with_args.is_empty() {
            let err_msg = "Command executor configured with an empty command list.".to_string();
            eprintln!("ERROR: {}", err_msg);
            return ExecutionStatus::Other(err_msg);
        }
        let mut cmd = Command::new(&self.config.command_with_args[0]);
        if self.config.command_with_args.len() > 1 {
            cmd.args(&self.config.command_with_args[1..]);
        }

        if let Some(cwd) = &self.config.working_directory {
            cmd.current_dir(cwd);
        }
        let mut temp_file_guard: Option<NamedTempFile> = None;

        match &self.config.input_delivery_mode {
            InputDeliveryMode::StdIn => {
                cmd.stdin(Stdio::piped());
            }
            InputDeliveryMode::File(arg_template_str) => {
                match NamedTempFile::new() {
                    Ok(mut temp_file) => {
                        if let Err(e) = temp_file.write_all(input.as_bytes()) {
                            let err_msg = format!(
                                "Failed to write input to temporary file {:?}: {}",
                                temp_file.path(),
                                e
                            );
                            eprintln!("ERROR: {}", err_msg);
                            return ExecutionStatus::Other(err_msg);
                        }
                        // Keep temp_file object alive until process finishes.
                        // The path needs to be valid UTF-8 to be used in Command::arg.
                        if let Some(path_str) = temp_file.path().to_str() {
                            let final_arg = arg_template_str.replace("{}", path_str);
                            // Args should be added one by one if the template might contain spaces
                            // that are part of a single argument. For now, assuming simple replacement.
                            // If complex arg structures are needed, the template itself should produce multiple args.
                            cmd.arg(final_arg);
                            temp_file_guard = Some(temp_file);
                        } else {
                            let err_msg = format!(
                                "Temporary file path {:?} is not valid UTF-8",
                                temp_file.path()
                            );
                            eprintln!("ERROR: {}", err_msg);
                            return ExecutionStatus::Other(err_msg);
                        }
                    }
                    Err(e) => {
                        let err_msg = format!("Failed to create temporary file for input: {}", e);
                        eprintln!("ERROR: {}", err_msg);
                        return ExecutionStatus::Other(err_msg);
                    }
                }
            }
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // 3. Spawn the child process
        let mut child_process = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                let error_msg = format!(
                    "Failed to spawn command '{:?}': {}",
                    self.config.command_with_args, e
                );
                eprintln!("ERROR: {}", error_msg);
                // Attempt to run post_exec for observers even on spawn failure
                let status_on_fail = ExecutionStatus::Other(error_msg.clone());
                for obs_on_fail in observers.iter_mut() {
                    if let Err(e_obs) = obs_on_fail.post_exec(
                        &status_on_fail,
                        None::<&dyn Any>,
                        Some(input as &dyn Input),
                    ) {
                        eprintln!(
                            "ERROR: Observer '{}' post_exec failed after spawn error: {}",
                            obs_on_fail.name(),
                            e_obs
                        );
                    }
                }
                return status_on_fail;
            }
        };

        // 4. Write to stdin if configured (after spawn, before wait)
        if let InputDeliveryMode::StdIn = self.config.input_delivery_mode {
            if let Some(mut child_stdin) = child_process.stdin.take() {
                if let Err(e) = child_stdin.write_all(input.as_bytes()) {
                    eprintln!("ERROR: Error writing to child stdin: {}. Killing child.", e);
                    let _ = child_process.kill();
                    let _ = child_process.wait();
                    return ExecutionStatus::Other(format!(
                        "Failed to write to target's stdin: {}",
                        e
                    ));
                }
            } else {
                let _ = child_process.kill();
                let _ = child_process.wait();
                return ExecutionStatus::Other(
                    "Child stdin was not available after piping.".to_string(),
                );
            }
        }

        // 5. Wait for the child process with timeout (capturing stdout/stderr is tricky with this approach)
        // A more robust method for capturing output with timeout involves `child.wait_with_output()`
        // in a separate thread, or using crates like `subprocess` or `duct`.
        let exit_status_result =
            self.run_and_wait_with_timeout(child_process, self.config.execution_timeout);

        let collected_process_output = ProcessOutput::default();

        // Determine execution status based on exit_status_result
        let final_status = match exit_status_result {
            Ok(status) => {
                let mut process_data_for_observer = ProcessOutput {
                    exit_code: status.code(),
                    ..Default::default()
                };

                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    process_data_for_observer.signal_code = status.signal();
                }

                if status.success() {
                    ExecutionStatus::Ok
                } else {
                    let crash_desc = if let Some(code) = process_data_for_observer.exit_code {
                        format!("Target exited with non-zero code: {}", code)
                    } else if cfg!(unix) && process_data_for_observer.signal_code.is_some() {
                        format!(
                            "Target terminated by signal: {}",
                            process_data_for_observer.signal_code.unwrap()
                        )
                    } else {
                        "Target exited abnormally (unknown reason)".to_string()
                    };
                    ExecutionStatus::Crash(crash_desc)
                }
            }
            Err(status_from_wait_logic) => status_from_wait_logic,
        };

        // Drop temp file if one was used
        drop(temp_file_guard);

        // 6. Call post_exec on all observers
        let mut overall_status = final_status;
        let mut post_exec_observer_error_msg: Option<String> = None;

        for obs in observers.iter_mut() {
            if let Err(e) = obs.post_exec(
                &overall_status,
                Some(&collected_process_output as &dyn Any),
                Some(input as &dyn Input),
            ) {
                let error_msg = format!("Observer '{}' post_exec failed: {}", obs.name(), e);
                eprintln!("ERROR: {}", error_msg);
                if post_exec_observer_error_msg.is_none() {
                    post_exec_observer_error_msg = Some(error_msg);
                }
            }
        }

        // If the primary status was Ok, but an observer failed in post_exec,
        // elevate the status to ObserverError.
        // Critical statuses like Crash or Timeout take precedence over post_exec observer errors.
        if overall_status == ExecutionStatus::Ok {
            if let Some(observer_err_msg) = post_exec_observer_error_msg {
                overall_status = ExecutionStatus::ObserverError(observer_err_msg);
            }
        } else if post_exec_observer_error_msg.is_some()
            && !matches!(overall_status, ExecutionStatus::ObserverError(_))
        {
            eprintln!(
                "INFO: Observer error during post_exec occurred with primary status: {:?}",
                overall_status
            );
        }

        overall_status
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::{MockCoverageObserver, NoOpObserver, Observer};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[cfg(test)]
    mod in_process_executor_tests {
        use super::*;

        fn simple_ok_harness(_data: &[u8]) {
            // No operation
        }

        fn panicking_harness(data: &[u8]) {
            if !data.is_empty() && data[0] == 0xFF {
                panic!("Test panic triggered by input 0xFF!");
            }
        }

        #[derive(Default)]
        struct TestObserverWithState {
            pre_exec_called: AtomicBool,
            post_exec_called: AtomicBool,
            reset_called: AtomicBool,
            input_byte_sum_on_post: AtomicUsize,
            name: &'static str,
        }

        impl TestObserverWithState {
            fn new(name: &'static str) -> Self {
                Self {
                    name,
                    ..Default::default()
                }
            }
        }

        impl Observer for TestObserverWithState {
            fn name(&self) -> &'static str {
                self.name
            }
            fn pre_exec(&mut self) -> Result<(), anyhow::Error> {
                self.reset()?;
                self.pre_exec_called.store(true, Ordering::SeqCst);
                Ok(())
            }
            fn post_exec(
                &mut self,
                _s: &ExecutionStatus,
                _to: Option<&dyn Any>,
                i_opt: Option<&dyn Input>,
            ) -> Result<(), anyhow::Error> {
                self.post_exec_called.store(true, Ordering::SeqCst);
                if let Some(inp) = i_opt {
                    self.input_byte_sum_on_post.store(
                        inp.as_bytes().iter().map(|&b| b as usize).sum(),
                        Ordering::SeqCst,
                    );
                }
                Ok(())
            }
            fn reset(&mut self) -> Result<(), anyhow::Error> {
                self.reset_called.store(true, Ordering::SeqCst);
                self.post_exec_called.store(false, Ordering::SeqCst);
                self.input_byte_sum_on_post.store(0, Ordering::SeqCst);
                Ok(())
            }

            fn serialize_data(&self) -> Option<Vec<u8>> {
                None
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn Any {
                self
            }
        }

        #[test]
        fn in_process_executor_runs_simple_harness_successfully() {
            let mut executor = InProcessExecutor::new(simple_ok_harness);
            let input_data: Vec<u8> = vec![1, 2, 3];
            let mut observer = TestObserverWithState::new("obs1");

            assert!(!observer.pre_exec_called.load(Ordering::Relaxed));
            assert!(!observer.reset_called.load(Ordering::Relaxed));

            let mut observers_list: [&mut dyn Observer; 1] = [&mut observer];
            let status = executor.execute_sync(&input_data, &mut observers_list);

            assert_eq!(status, ExecutionStatus::Ok);
            assert!(
                observer.pre_exec_called.load(Ordering::SeqCst),
                "Observer pre_exec_called should be true after execution"
            );
            assert!(observer.post_exec_called.load(Ordering::SeqCst));
            assert!(
                observer.reset_called.load(Ordering::SeqCst),
                "Observer reset_called_flag (from pre_exec) should be true"
            );
            assert_eq!(observer.input_byte_sum_on_post.load(Ordering::SeqCst), 6);
        }

        #[test]
        fn in_process_executor_catches_harness_panic() {
            let mut executor = InProcessExecutor::new(panicking_harness);
            let mut observer = NoOpObserver;
            let mut observers_list: [&mut dyn Observer; 1] = [&mut observer];

            let crashing_input: Vec<u8> = vec![0xFF, 0xAA, 0xBB]; // Starts with 0xFF
            let status = executor.execute_sync(&crashing_input, &mut observers_list);

            match status {
                ExecutionStatus::Crash(msg) => {
                    assert!(
                        msg.contains("Test panic triggered by input 0xFF!"),
                        "Crash message mismatch. Got: {}",
                        msg
                    );
                }
                _ => panic!("Expected ExecutionStatus::Crash, got {:?}", status),
            }
        }

        #[test]
        fn in_process_executor_handles_observer_pre_exec_failure() {
            struct FailingPreObserver;
            impl Observer for FailingPreObserver {
                fn name(&self) -> &'static str {
                    "FailingPre"
                }
                fn pre_exec(&mut self) -> Result<(), anyhow::Error> {
                    Err(anyhow::anyhow!("pre_exec failed"))
                }
                fn post_exec(
                    &mut self,
                    _: &ExecutionStatus,
                    _: Option<&dyn Any>,
                    _: Option<&dyn Input>,
                ) -> Result<(), anyhow::Error> {
                    Ok(())
                }
                fn reset(&mut self) -> Result<(), anyhow::Error> {
                    Ok(())
                }
                fn serialize_data(&self) -> Option<Vec<u8>> {
                    None
                }
                fn as_any(&self) -> &dyn Any {
                    self
                }
                fn as_any_mut(&mut self) -> &mut dyn Any {
                    self
                }
            }
            let mut executor = InProcessExecutor::new(simple_ok_harness);
            let input_data: Vec<u8> = vec![1];
            let mut failing_observer = FailingPreObserver;
            let mut observers: [&mut dyn Observer; 1] = [&mut failing_observer];

            let status = executor.execute_sync(&input_data, &mut observers);
            match status {
                ExecutionStatus::ObserverError(msg) => assert!(msg.contains("pre_exec failed")),
                _ => panic!("Expected ObserverError, got {:?}", status),
            }
        }

        #[test]
        fn in_process_executor_handles_observer_post_exec_failure_when_harness_ok() {
            struct FailingPostObserver {
                post_exec_called_count: usize,
            }
            impl Observer for FailingPostObserver {
                fn name(&self) -> &'static str {
                    "FailingPost"
                }
                fn pre_exec(&mut self) -> Result<(), anyhow::Error> {
                    Ok(())
                }
                fn post_exec(
                    &mut self,
                    _: &ExecutionStatus,
                    _: Option<&dyn Any>,
                    _: Option<&dyn Input>,
                ) -> Result<(), anyhow::Error> {
                    self.post_exec_called_count += 1;
                    Err(anyhow::anyhow!("post_exec failed"))
                }
                fn reset(&mut self) -> Result<(), anyhow::Error> {
                    Ok(())
                }
                fn serialize_data(&self) -> Option<Vec<u8>> {
                    None
                }
                fn as_any(&self) -> &dyn Any {
                    self
                }
                fn as_any_mut(&mut self) -> &mut dyn Any {
                    self
                }
            }
            let mut executor = InProcessExecutor::new(simple_ok_harness);
            let input_data: Vec<u8> = vec![1];
            let mut failing_observer = FailingPostObserver {
                post_exec_called_count: 0,
            };
            let mut observers: [&mut dyn Observer; 1] = [&mut failing_observer];

            let status = executor.execute_sync(&input_data, &mut observers);
            match status {
                ExecutionStatus::ObserverError(msg) => {
                    assert!(msg.contains("post_exec failed"));
                    assert_eq!(
                        failing_observer.post_exec_called_count, 1,
                        "post_exec should have been called once"
                    );
                }
                _ => panic!(
                    "Expected ObserverError for post_exec failure, got {:?}",
                    status
                ),
            }
        }

        #[test]
        fn in_process_executor_reports_crash_even_if_post_exec_fails() {
            struct FailingPostObserver {
                post_exec_called_count: usize,
            }
            impl Observer for FailingPostObserver {
                fn name(&self) -> &'static str {
                    "FailingPostAfterCrash"
                }
                fn pre_exec(&mut self) -> Result<(), anyhow::Error> {
                    Ok(())
                }
                fn post_exec(
                    &mut self,
                    status: &ExecutionStatus,
                    _: Option<&dyn Any>,
                    _: Option<&dyn Input>,
                ) -> Result<(), anyhow::Error> {
                    self.post_exec_called_count += 1;
                    assert!(
                        matches!(status, ExecutionStatus::Crash(_)),
                        "Observer should see Crash status in post_exec"
                    );
                    Err(anyhow::anyhow!("post_exec failed after crash"))
                }
                fn reset(&mut self) -> Result<(), anyhow::Error> {
                    Ok(())
                }
                fn serialize_data(&self) -> Option<Vec<u8>> {
                    None
                }
                fn as_any(&self) -> &dyn Any {
                    self
                }
                fn as_any_mut(&mut self) -> &mut dyn Any {
                    self
                }
            }
            let mut executor = InProcessExecutor::new(panicking_harness);
            let mut failing_observer = FailingPostObserver {
                post_exec_called_count: 0,
            };
            let mut observers: [&mut dyn Observer; 1] = [&mut failing_observer];
            let crashing_input: Vec<u8> = vec![0xFF]; // Triggers panic

            let status = executor.execute_sync(&crashing_input, &mut observers);
            match status {
                ExecutionStatus::Crash(msg) => {
                    assert!(msg.contains("Test panic triggered"));
                    assert_eq!(
                        failing_observer.post_exec_called_count, 1,
                        "post_exec should have been called once, even after crash"
                    );
                }
                _ => panic!(
                    "Expected Crash status, even with post_exec failure, got {:?}",
                    status
                ),
            }
        }
    }

    #[cfg(test)]
    mod command_executor_tests {
        use super::*;
        use std::env;

        // Helper to get path to test_targets
        fn get_test_target_path(target_name: &str) -> PathBuf {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test_targets")
                .join(target_name)
        }

        #[test]
        fn cmd_exec_successful_run_with_stdin_delivery() {
            let target_path = get_test_target_path("test_target_ok.sh");
            if !target_path.exists() {
                panic!("Test target script missing: {:?}", target_path);
            }

            let config = CommandExecutorConfig {
                command_with_args: vec![target_path.to_string_lossy().into_owned()],
                input_delivery_mode: InputDeliveryMode::StdIn,
                execution_timeout: Duration::from_secs(2),
                working_directory: None,
            };
            let mut executor = CommandExecutor::new(config);
            let input_data: Vec<u8> = b"hello_stdin".to_vec();
            let mut no_op_observer = NoOpObserver;
            let mut observers: [&mut dyn Observer; 1] = [&mut no_op_observer];

            let status = executor.execute_sync(&input_data, &mut observers);
            assert_eq!(status, ExecutionStatus::Ok, "Expected successful execution");
        }

        #[test]
        fn cmd_exec_detects_crash_exit_code() {
            let target_path = get_test_target_path("test_target_exit_code_crash.sh");
            if !target_path.exists() {
                panic!("Test script missing: {:?}", target_path);
            }
            let config = CommandExecutorConfig {
                command_with_args: vec![
                    target_path.to_string_lossy().into_owned(),
                    "77".to_string(),
                ], // Script expects arg for exit code
                input_delivery_mode: InputDeliveryMode::StdIn,
                execution_timeout: Duration::from_secs(1),
                working_directory: None,
            };
            let mut executor = CommandExecutor::new(config);
            let mut obs = NoOpObserver;
            let mut observers: [&mut dyn Observer; 1] = [&mut obs];
            let status = executor.execute_sync(&Vec::new(), &mut observers);
            match status {
                // Ensure the script actually exits with 77 after permissions are fixed
                ExecutionStatus::Crash(desc) => assert!(
                    desc.contains("exited with non-zero code: 77"),
                    "Description was: {}",
                    desc
                ),
                _ => panic!("Expected Crash with code 77, got {:?}", status),
            }
        }

        #[cfg(unix)]
        #[test]
        fn cmd_exec_detects_crash_by_signal() {
            let target_path = get_test_target_path("test_target_signal_crash.sh");
            if !target_path.exists() {
                panic!("Test script missing: {:?}", target_path);
            }
            let config = CommandExecutorConfig {
                command_with_args: vec![target_path.to_string_lossy().into_owned()],
                input_delivery_mode: InputDeliveryMode::StdIn,
                execution_timeout: Duration::from_secs(2),
                working_directory: None,
            };
            let mut executor = CommandExecutor::new(config);
            let mut obs = NoOpObserver;
            let mut observers: [&mut dyn Observer; 1] = [&mut obs];
            let status = executor.execute_sync(&Vec::new(), &mut observers);
            match status {
                ExecutionStatus::Crash(desc) => {
                    // Be a bit flexible with signal description, as it can vary slightly
                    // SIGSEGV is 11, SIGABRT is 6.
                    let sigsegv = desc.contains("signal: 11") || desc.contains("Signal 11");
                    let sigabrt = desc.contains("signal: 6") || desc.contains("Signal 6");
                    assert!(
                        sigsegv || sigabrt,
                        "Expected crash by SIGSEGV (11) or SIGABRT (6). Got: {}",
                        desc
                    );
                }
                _ => panic!("Expected Crash by signal, got {:?}", status),
            }
        }
        #[test]
        fn cmd_exec_handles_timeout_correctly() {
            // This script sleeps for 5 seconds
            let target_path = get_test_target_path("test_target_timeout.sh");
            if !target_path.exists() {
                panic!("Test target script missing: {:?}", target_path);
            }

            let config = CommandExecutorConfig {
                command_with_args: vec![target_path.to_string_lossy().into_owned()],
                input_delivery_mode: InputDeliveryMode::StdIn,
                execution_timeout: Duration::from_millis(100), // But we timeout after 200ms
                working_directory: None,
            };
            let mut executor = CommandExecutor::new(config);
            let input_data: Vec<u8> = Vec::new();
            let mut no_op_observer = NoOpObserver;
            let mut observers: [&mut dyn Observer; 1] = [&mut no_op_observer];

            let status = executor.execute_sync(&input_data, &mut observers);
            assert_eq!(
                status,
                ExecutionStatus::Timeout,
                "Expected execution to time out"
            );
        }

        #[test]
        fn cmd_exec_input_delivery_via_file() {
            let target_path = get_test_target_path("test_target_reads_file.sh");
            if !target_path.exists() {
                panic!("Test target script missing: {:?}", target_path);
            }

            let config = CommandExecutorConfig {
                command_with_args: vec![target_path.to_string_lossy().into_owned()],
                input_delivery_mode: InputDeliveryMode::File("{}".to_string()),
                execution_timeout: Duration::from_secs(1),
                working_directory: None,
            };

            let mut executor = CommandExecutor::new(config);
            let mut mock_obs_instance = MockCoverageObserver::new();
            let mut observers_list_for_exec: [&mut dyn Observer; 1] = [&mut mock_obs_instance];

            // Test 1: Expected to succeed
            let input_ok_content: Vec<u8> = b"EXPECTED_CONTENT_OK".to_vec();
            let status_ok = executor.execute_sync(&input_ok_content, &mut observers_list_for_exec);

            assert_eq!(
                status_ok,
                ExecutionStatus::Ok,
                "Expected Ok for correct file content"
            );

            let observer_trait_obj_ok: &mut dyn Observer = observers_list_for_exec[0];

            if status_ok == ExecutionStatus::Ok {
                match observer_trait_obj_ok.serialize_data() {
                    Some(observed_hash_data) => {
                        assert_eq!(
                            observed_hash_data,
                            md5::compute(input_ok_content.as_bytes()).0.to_vec(),
                            "Observed hash for OK case does not match input hash"
                        );
                    }
                    None => {
                        panic!(
                            "MockCoverageObserver (via slice) did not serialize data after successful OK execution. Hash was None."
                        );
                    }
                }
            }

            // Test 2: Expected to "crash" (exit with code 1)
            let input_fail_content: Vec<u8> = b"UNEXPECTED_FAIL_CONTENT".to_vec();

            observers_list_for_exec[0]
                .reset()
                .expect("Observer reset failed");

            let status_fail =
                executor.execute_sync(&input_fail_content, &mut observers_list_for_exec);
            match status_fail {
                ExecutionStatus::Crash(desc) => {
                    assert!(
                        desc.contains("exited with non-zero code: 1"),
                        "Expected script to exit 1 for wrong content, got: {}",
                        desc
                    );
                }
                _ => panic!(
                    "Expected Crash status for incorrect file content, got {:?}",
                    status_fail
                ),
            }

            let observer_trait_obj_fail: &mut dyn Observer = observers_list_for_exec[0];

            match observer_trait_obj_fail.serialize_data() {
                Some(observed_hash_data_fail) => {
                    assert_eq!(
                        observed_hash_data_fail,
                        md5::compute(input_fail_content.as_bytes()).0.to_vec(),
                        "Observed hash for FAIL case does not match input hash"
                    );
                }
                None => {
                    panic!(
                        "MockCoverageObserver (via slice) did not serialize data after FAIL execution. Hash was None."
                    );
                }
            }
        }

        #[test]
        fn cmd_exec_handles_invalid_command_path() {
            let config = CommandExecutorConfig {
                command_with_args: vec!["./this_command_does_not_exist_ever_12345.sh".to_string()],
                input_delivery_mode: InputDeliveryMode::StdIn,
                execution_timeout: Duration::from_secs(1),
                working_directory: None,
            };
            let mut executor = CommandExecutor::new(config);
            let input_data: Vec<u8> = Vec::new();
            let mut no_op_observer = NoOpObserver;
            let mut observers: [&mut dyn Observer; 1] = [&mut no_op_observer];

            let status = executor.execute_sync(&input_data, &mut observers);
            match status {
                ExecutionStatus::Other(msg) => {
                    assert!(
                        msg.contains("Failed to spawn command"),
                        "Error message should indicate spawn failure. Got: {}",
                        msg
                    );
                    // Underlying OS error might be "No such file or directory" or similar.
                }
                _ => panic!(
                    "Expected Other status for invalid command, got {:?}",
                    status
                ),
            }
        }
    }
}
