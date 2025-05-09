use crate::executor::ExecutionStatus;
use std::any::Any;

pub trait Observer: Send + Sync {
    fn name(&self) -> &'static str;
    fn pre_exec(&mut self) -> Result<(), anyhow::Error>;
    fn post_exec(
        &mut self,
        status: &ExecutionStatus,
        target_output: Option<&dyn Any>,
    ) -> Result<(), anyhow::Error>;
    fn reset(&mut self) -> Result<(), anyhow::Error>;
    fn serialize_data(&self) -> Option<Vec<u8>>;
}

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
