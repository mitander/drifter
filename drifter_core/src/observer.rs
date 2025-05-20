use crate::executor::ExecutionStatus;
use crate::input::Input;
use md5;
use std::any::Any;

/// An `Observer` is a component that monitors the execution of a target program
/// with a given input and collects data about that execution.
///
/// Observers are invoked by an `Executor` before (`pre_exec`) and after (`post_exec`)
/// the target is run. They can gather various forms of data, such as code coverage
/// (e.g., via `SanitizerCoverageObserver`), execution time, or custom application-specific metrics.
///
/// The data collected by observers (`serialize_data`) is then typically consumed by
/// `Feedback` mechanisms to determine if an input is "interesting" and should be
/// added to the corpus.
pub trait Observer: Send + Sync {
    /// Returns a static string name identifying the observer.
    ///
    /// This name is crucial for `Feedback` components to locate and interpret
    /// the data produced by this specific observer from a collection of observer data.
    fn name(&self) -> &'static str;

    /// Called by the `Executor` immediately before the target program is executed.
    ///
    /// This method is typically used to:
    /// 1. Reset any internal state of the observer from previous executions (often by calling `self.reset()`).
    /// 2. Perform any setup required for data collection (e.g., enabling instrumentation).
    ///
    /// # Returns
    /// `Ok(())` on success, or an `anyhow::Error` if setup fails.
    fn pre_exec(&mut self) -> Result<(), anyhow::Error>;

    /// Called by the `Executor` immediately after the target program has finished execution.
    ///
    /// This method is where the observer collects data based on the execution outcome.
    ///
    /// # Arguments
    /// * `status`: The `ExecutionStatus` reported by the `Executor` (e.g., Ok, Crash, Timeout).
    /// * `target_output`: Optional, arbitrary data captured from the target's execution,
    ///   such as stdout/stderr or custom data from another observer. This can be `None`.
    /// * `input_opt`: The `Input` that was just executed, if available to the executor.
    ///   This might be `None` in some edge cases or if not provided.
    ///
    /// # Returns
    /// `Ok(())` on success, or an `anyhow::Error` if data collection or processing fails.
    fn post_exec(
        &mut self,
        status: &ExecutionStatus,
        target_output: Option<&dyn Any>,
        input_opt: Option<&dyn Input>,
    ) -> Result<(), anyhow::Error>;

    /// Resets the internal state of the observer.
    ///
    /// This is essential to ensure that data from one execution does not interfere
    /// with data from subsequent executions. Typically called from `pre_exec`.
    ///
    /// # Returns
    /// `Ok(())` on success, or an `anyhow::Error` if resetting fails.
    fn reset(&mut self) -> Result<(), anyhow::Error>;

    /// Serializes the data collected by this observer during the last execution cycle.
    ///
    /// The format of the `Vec<u8>` is specific to the observer's implementation.
    /// `Feedback` components that consume this data must understand this format.
    ///
    /// # Returns
    /// `Some(Vec<u8>)` containing the serialized data if data was collected,
    /// or `None` if no relevant data is available or if the observer doesn't produce serializable data.
    fn serialize_data(&self) -> Option<Vec<u8>>;

    /// Returns the observer as a `&dyn Any` reference.
    ///
    /// This enables downcasting to a concrete observer type if needed,
    /// though direct downcasting is often discouraged in favor of well-defined traits.
    fn as_any(&self) -> &dyn Any;

    /// Returns the observer as a mutable `&mut dyn Any` reference.
    ///
    /// Similar to `as_any`, but allows mutable downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// A `NoOpObserver` is an observer that performs no actions
/// and collects no data.
///
/// It can be used as a default placeholder when no specific observation is needed,
/// or for testing basic fuzzer execution flow without the overhead of actual observers.
#[derive(Default, Debug, Clone, Copy)]
pub struct NoOpObserver;

impl Observer for NoOpObserver {
    fn name(&self) -> &'static str {
        "NoOpObserver"
    }

    fn pre_exec(&mut self) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn post_exec(
        &mut self,
        _status: &ExecutionStatus,
        _target_output: Option<&dyn Any>,
        _input_opt: Option<&dyn Input>,
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

/// A `MockCoverageObserver` simulates coverage collection by computing an MD5 hash
/// of the input processed during `post_exec`.
///
/// This observer is primarily intended for testing the fuzzer's feedback loop
/// (specifically components like `MockBitmapCoverageFeedback`) when actual code
/// coverage mechanisms (like sanitizers) are not available or not yet integrated.
/// The "coverage data" it produces via `serialize_data` is the 16-byte MD5 hash.
#[derive(Default, Debug, Clone)]
pub struct MockCoverageObserver {
    /// Stores the MD5 hash ([u8; 16]) of the most recently processed input.
    /// `None` if no input has been processed since the last reset.
    pub current_input_hash: Option<[u8; 16]>,
}

impl MockCoverageObserver {
    /// Creates a new, default `MockCoverageObserver`.
    /// The `current_input_hash` will be `None`.
    pub fn new() -> Self {
        Default::default()
    }
}

impl Observer for MockCoverageObserver {
    fn name(&self) -> &'static str {
        "MockCoverageObserver"
    }

    /// Resets the `current_input_hash` to `None`. This is called before each execution.
    fn pre_exec(&mut self) -> Result<(), anyhow::Error> {
        self.reset()
    }

    /// If `input_opt` is `Some`, computes its MD5 hash and stores it in `current_input_hash`.
    /// The `status` and `target_output` are ignored by this mock observer.
    fn post_exec(
        &mut self,
        _status: &ExecutionStatus,
        _target_output: Option<&dyn Any>,
        input_opt: Option<&dyn Input>,
    ) -> Result<(), anyhow::Error> {
        if let Some(input_ref) = input_opt {
            let digest = md5::compute(input_ref.as_bytes());
            self.current_input_hash = Some(digest.0);
        } else {
            self.current_input_hash = None;
        }
        Ok(())
    }

    /// Clears the stored `current_input_hash`, setting it to `None`.
    fn reset(&mut self) -> Result<(), anyhow::Error> {
        self.current_input_hash = None;
        Ok(())
    }

    /// Serializes the `current_input_hash` (if `Some`) into a `Vec<u8>`.
    ///
    /// # Returns
    /// `Some(Vec<u8>)` containing the 16-byte MD5 hash if an input was processed,
    /// or `None` if no hash is available (e.g., after a reset or if no input was provided
    /// to `post_exec`).
    fn serialize_data(&self) -> Option<Vec<u8>> {
        self.current_input_hash
            .as_ref()
            .map(|hash_array_ref| hash_array_ref.to_vec())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::ExecutionStatus;

    #[test]
    fn no_op_observer_behaves_as_expected() {
        let mut observer = NoOpObserver;
        assert_eq!(
            observer.name(),
            "NoOpObserver",
            "Name should be 'NoOpObserver'"
        );
        assert!(
            observer.pre_exec().is_ok(),
            "pre_exec should always return Ok"
        );

        let dummy_status = ExecutionStatus::Ok;
        let dummy_input: Vec<u8> = vec![1, 2, 3];
        assert!(
            observer
                .post_exec(&dummy_status, None, Some(&dummy_input))
                .is_ok(),
            "post_exec should always return Ok"
        );
        assert!(observer.reset().is_ok(), "reset should always return Ok");
        assert!(
            observer.serialize_data().is_none(),
            "serialize_data should always return None"
        );

        let _ = observer.as_any().downcast_ref::<NoOpObserver>().unwrap();
        let _ = observer
            .as_any_mut()
            .downcast_mut::<NoOpObserver>()
            .unwrap();
    }

    #[test]
    fn mock_coverage_observer_correctly_handles_state_and_data() {
        let mut observer = MockCoverageObserver::new();
        assert_eq!(
            observer.name(),
            "MockCoverageObserver",
            "Name should be 'MockCoverageObserver'"
        );

        assert!(
            observer.current_input_hash.is_none(),
            "Initially, current_input_hash should be None"
        );
        assert!(
            observer.serialize_data().is_none(),
            "Initially, serialize_data should return None"
        );

        observer.current_input_hash = Some(md5::compute(b"stale_data").0);
        assert!(observer.pre_exec().is_ok(), "pre_exec should succeed");
        assert!(
            observer.current_input_hash.is_none(),
            "pre_exec should reset current_input_hash to None"
        );
        assert!(
            observer.serialize_data().is_none(),
            "After pre_exec, serialize_data should be None"
        );

        let input_data_1: Vec<u8> = b"test_input_alpha".to_vec();
        let dummy_status_ok = ExecutionStatus::Ok;
        assert!(
            observer
                .post_exec(&dummy_status_ok, None, Some(&input_data_1))
                .is_ok(),
            "post_exec with input_data_1 should succeed"
        );

        let expected_hash_1 = md5::compute(b"test_input_alpha").0;
        assert_eq!(
            observer.current_input_hash,
            Some(expected_hash_1),
            "current_input_hash should be set to hash of input_data_1"
        );
        assert_eq!(
            observer.serialize_data(),
            Some(expected_hash_1.to_vec()),
            "serialize_data should return the hash of input_data_1"
        );

        assert!(
            observer.pre_exec().is_ok(),
            "pre_exec for next cycle should succeed"
        );
        assert!(
            observer.current_input_hash.is_none(),
            "Hash should be reset by pre_exec"
        );

        let input_data_2: Vec<u8> = b"test_input_beta_longer".to_vec();
        assert!(
            observer
                .post_exec(&dummy_status_ok, None, Some(&input_data_2))
                .is_ok(),
            "post_exec with input_data_2 should succeed"
        );
        let expected_hash_2 = md5::compute(b"test_input_beta_longer").0;
        assert_eq!(
            observer.current_input_hash,
            Some(expected_hash_2),
            "Hash should update to input_data_2's hash"
        );
        assert_eq!(
            observer.serialize_data(),
            Some(expected_hash_2.to_vec()),
            "Serialized data should be hash of input_data_2"
        );

        assert!(
            observer.pre_exec().is_ok(),
            "pre_exec before None input test"
        );
        assert!(
            observer.post_exec(&dummy_status_ok, None, None).is_ok(),
            "post_exec with None input should succeed"
        );
        assert!(
            observer.current_input_hash.is_none(),
            "current_input_hash should be None if post_exec received None input"
        );
        assert!(
            observer.serialize_data().is_none(),
            "serialize_data should be None if hash is None"
        );

        observer.current_input_hash = Some(md5::compute(b"another_hash_to_clear").0);
        assert!(observer.reset().is_ok(), "Explicit reset should succeed");
        assert!(
            observer.current_input_hash.is_none(),
            "reset() should clear current_input_hash"
        );
        assert!(
            observer.serialize_data().is_none(),
            "serialize_data after reset should be None"
        );
    }
}
