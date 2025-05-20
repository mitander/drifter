use crate::executor::ExecutionStatus;
use crate::input::Input;
use md5;
use std::any::Any;

/// Default severity level for crashes detected by `CrashOracle`.
/// Higher values might indicate more severe issues.
const DEFAULT_CRASH_SEVERITY: u8 = 10;

/// Represents a potential bug or interesting finding identified by an `Oracle`.
#[derive(Debug)]
pub struct BugReport<I: Input> {
    /// The specific input that triggered this bug report.
    pub input: I,
    /// A human-readable description of the bug or finding.
    pub description: String,
    /// An identifying hash of the input (e.g., MD5), useful for deduplication or tracking.
    pub input_hash: String,
    /// A numerical representation of the bug's severity.
    /// The scale and meaning are defined by the specific oracle or fuzzer configuration.
    pub severity: u8,
}

/// An `Oracle` examines the outcome of a target's execution to determine if a bug has occurred.
///
/// Oracles are crucial for identifying crashes, hangs, sanitizer violations,
/// or other undesirable behaviors.
pub trait Oracle<I: Input>: Send + Sync {
    /// Examines the execution status and any target output for a given input
    /// to determine if a bug should be reported.
    ///
    /// # Arguments
    /// * `input`: A reference to the `Input` that was executed.
    /// * `status`: The `ExecutionStatus` returned by the `Executor` after running the input.
    /// * `target_output`: Optional, arbitrary data captured from the target's execution
    ///   (e.g., stdout, stderr, or custom observer data). This can be `None` if not
    ///   captured or not relevant to this oracle.
    ///
    /// # Returns
    /// An `Option<BugReport<I>>`. Returns `Some(BugReport)` if a bug is detected,
    /// otherwise `None`.
    fn examine(
        &self,
        input: &I,
        status: &ExecutionStatus,
        target_output: Option<&dyn Any>,
    ) -> Option<BugReport<I>>;
}

/// A simple `Oracle` that reports a bug if the `ExecutionStatus` is `Crash`.
///
/// This is a common, basic oracle used in many fuzzers.
#[derive(Debug, Default)]
pub struct CrashOracle;

impl CrashOracle {
    /// Creates a new `CrashOracle`.
    pub fn new() -> Self {
        CrashOracle
    }
}

impl<I> Oracle<I> for CrashOracle
where
    I: Input + Clone,
{
    /// Reports a bug if the execution `status` indicates a crash.
    /// The `target_output` is currently ignored by this specific oracle.
    fn examine(
        &self,
        input: &I,
        status: &ExecutionStatus,
        _target_output: Option<&dyn Any>,
    ) -> Option<BugReport<I>> {
        match status {
            ExecutionStatus::Crash(description) => {
                let input_digest = md5::compute(input.as_bytes());
                Some(BugReport {
                    input: input.clone(),
                    description: description.clone(),
                    input_hash: format!("{:x}", input_digest),
                    severity: DEFAULT_CRASH_SEVERITY,
                })
            }
            _ => None, // No bug reported for non-crash statuses
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::ExecutionStatus;

    #[test]
    fn crash_oracle_detects_crash_and_creates_valid_report() {
        let oracle = CrashOracle::new();
        let input_data: Vec<u8> = vec![0xFF, 0xFE, 0xFD];
        let crash_description = "Test panic: Segmentation fault!".to_string();
        let crash_status = ExecutionStatus::Crash(crash_description.clone());

        let bug_report_option = oracle.examine(&input_data, &crash_status, None);
        assert!(
            bug_report_option.is_some(),
            "Oracle should detect a crash and return Some(BugReport)"
        );

        if let Some(report) = bug_report_option {
            assert_eq!(
                report.input, input_data,
                "Report input should match the original input"
            );
            assert_eq!(
                report.description, crash_description,
                "Report description should match the crash status description"
            );

            let expected_hash = format!("{:x}", md5::compute(input_data.as_bytes()));
            assert_eq!(
                report.input_hash, expected_hash,
                "Report input_hash should be the MD5 hex string of the input"
            );
            assert_eq!(
                report.severity, DEFAULT_CRASH_SEVERITY,
                "Report severity should be the default crash severity"
            );
        }
    }

    #[test]
    fn crash_oracle_ignores_ok_status() {
        let oracle = CrashOracle::new();
        let input_data: Vec<u8> = vec![0x01, 0x02, 0x03];
        let ok_status = ExecutionStatus::Ok;

        let bug_report_option = oracle.examine(&input_data, &ok_status, None);
        assert!(
            bug_report_option.is_none(),
            "Oracle should ignore Ok status and return None"
        );
    }

    #[test]
    fn crash_oracle_ignores_timeout_status() {
        let oracle = CrashOracle::new();
        let input_data: Vec<u8> = vec![0xAA, 0xBB, 0xCC];
        let timeout_status = ExecutionStatus::Timeout;

        let bug_report_option = oracle.examine(&input_data, &timeout_status, None);
        assert!(
            bug_report_option.is_none(),
            "Oracle should ignore Timeout status and return None"
        );
    }

    #[test]
    fn crash_oracle_ignores_other_non_crash_status() {
        let oracle = CrashOracle::new();
        let input_data: Vec<u8> = vec![0x11];
        let other_status = ExecutionStatus::Other("Some other issue".to_string());
        let bug_report_option = oracle.examine(&input_data, &other_status, None);
        assert!(
            bug_report_option.is_none(),
            "Oracle should ignore Other status and return None"
        );
    }
}
