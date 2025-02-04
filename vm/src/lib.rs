//! Virtual Machine low-level management

mod bits;
mod memory;
mod snapshot;
mod vm;
mod x64;

#[macro_use]
extern crate vmm_sys_util;

pub use memory::{Mapping, PagePermissions};
pub use snapshot::{
    SnapshotError, SnapshotInfo, SnapshotMapping, SnapshotModule, SnapshotRegisters,
};
pub use vm::{PageFaultDetail, Register, Vm, VmError, VmExit};
