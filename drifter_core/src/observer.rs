use crate::executor::ExecutionStatus;
use crate::input::Input;
use std::any::Any;

pub trait Observer: Send + Sync {
    fn name(&self) -> &'static str;
    fn pre_exec(&mut self) -> Result<(), anyhow::Error>;
    fn post_exec(
        &mut self,
        status: &ExecutionStatus,
        target_output: Option<&dyn Any>,
        input_opt: Option<&dyn Input>,
    ) -> Result<(), anyhow::Error>;
    fn reset(&mut self) -> Result<(), anyhow::Error>;
    fn serialize_data(&self) -> Option<Vec<u8>>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[derive(Default)]
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

#[derive(Default)]
pub struct MockCoverageObserver {
    current_input_hash: Option<[u8; 16]>,
}

impl MockCoverageObserver {
    pub fn new() -> Self {
        Default::default()
    }
}

impl Observer for MockCoverageObserver {
    fn name(&self) -> &'static str {
        "MockCoverageObserver"
    }

    fn pre_exec(&mut self) -> Result<(), anyhow::Error> {
        self.reset()
    }

    fn post_exec(
        &mut self,
        _status: &ExecutionStatus,
        _target_output: Option<&dyn Any>,
        input_opt: Option<&dyn Input>,
    ) -> Result<(), anyhow::Error> {
        if let Some(input_trait_obj) = input_opt {
            let hash = md5::compute(input_trait_obj.as_bytes());
            self.current_input_hash = Some(hash.0);
        }
        Ok(())
    }

    fn reset(&mut self) -> Result<(), anyhow::Error> {
        self.current_input_hash = None;
        Ok(())
    }

    fn serialize_data(&self) -> Option<Vec<u8>> {
        self.current_input_hash
            .map(|hash_array| hash_array.to_vec())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
