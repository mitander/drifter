use crate::executor::ExecutionStatus;
use crate::input::Input;
use std::any::Any;

#[derive(Debug)]
pub struct BugReport<I: Input> {
    pub input: I,
    pub description: String,
    pub hash: String,
    pub severity: u8,
}

pub trait Oracle<I: Input>: Send + Sync {
    fn examine(
        &self,
        input: &I,
        status: &ExecutionStatus,
        target_output: Option<&dyn Any>,
    ) -> Option<BugReport<I>>;
}

pub struct CrashOracle;

impl<I: Input> Oracle<I> for CrashOracle {
    fn examine(
        &self,
        input: &I,
        status: &ExecutionStatus,
        _target_output: Option<&dyn Any>,
    ) -> Option<BugReport<I>> {
        match status {
            ExecutionStatus::Crash(description) => Some(BugReport {
                input: input.clone(),
                description: description.clone(),
                hash: format!("{:x}", md5::compute(input.as_bytes())),
                severity: 10,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::ExecutionStatus;

    #[test]
    fn crash_oracle_detects_crash() {
        let oracle = CrashOracle;
        let input_data: Vec<u8> = vec![0xFF];
        let crash_status = ExecutionStatus::Crash("Test panic!".to_string());
        let bug_report = oracle.examine(&input_data, &crash_status, None);
        assert!(bug_report.is_some());
        let report = bug_report.unwrap();
        assert_eq!(report.input, input_data);
        assert_eq!(report.description, "Test panic!".to_string());
    }

    #[test]
    fn crash_oracle_ignores_ok() {
        let oracle = CrashOracle;
        let input_data: Vec<u8> = vec![0x01];
        let ok_status = ExecutionStatus::Ok;
        let bug_report = oracle.examine(&input_data, &ok_status, None);
        assert!(bug_report.is_none());
    }
}
