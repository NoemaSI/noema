pub mod error;
pub mod runtime;
pub mod vm;

pub use error::{Error, Result};
pub use runtime::ensure_runtime;
pub use vm::Vm;

pub use smolmachines::{
    ExecEvent, ExecOptions, ExecResult, ExecStream, MachineState, Mount, Port, ReadyOptions,
    RuntimeAssets,
};