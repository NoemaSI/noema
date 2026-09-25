use std::path::Path;

use smolmachines::{
    ExecOptions, ExecResult, ExecStream, Machine, MachineBuilder, MachineState, ReadyOptions,
};

use crate::{Result, ensure_runtime};

/// Lifecycle handle around a smolmachines microVM.
///
/// The underlying engine handle owns nothing: dropping a `Vm` neither stops
/// nor deletes the machine. Call [`Vm::stop`] or [`Vm::delete`] explicitly.
#[derive(Debug, Clone)]
pub struct Vm {
    machine: Machine,
}

impl Vm {
    /// Start describing a machine of this name.
    pub fn builder(name: impl Into<String>) -> MachineBuilder {
        Machine::builder(name)
    }

    /// Create a machine from a builder. Not started yet.
    pub fn create(builder: MachineBuilder) -> Result<Self> {
        ensure_runtime()?;
        let machine = builder.create()?;
        Ok(Self { machine })
    }

    /// Create and boot a machine, blocking until the guest agent is ready.
    pub fn spawn(builder: MachineBuilder) -> Result<Self> {
        let vm = Self::create(builder)?;
        vm.start()?;
        vm.wait_until_ready()?;
        Ok(vm)
    }

    /// Attach to an existing machine by name, starting it if it is stopped.
    pub fn attach(name: impl Into<String>) -> Result<Self> {
        ensure_runtime()?;
        let machine = Machine::connect(name)?;
        Ok(Self { machine })
    }

    /// Attach to an existing machine by name without starting it.
    pub fn inspect(name: impl Into<String>) -> Result<Self> {
        ensure_runtime()?;
        let machine = Machine::attach(name)?;
        Ok(Self { machine })
    }

    pub fn name(&self) -> &str {
        self.machine.name()
    }

    pub fn id(&self) -> &str {
        self.machine.id()
    }

    pub fn pid(&self) -> Option<i32> {
        self.machine.pid()
    }

    pub fn state(&self) -> MachineState {
        self.machine.state()
    }

    pub fn is_running(&self) -> bool {
        self.machine.is_running()
    }

    /// Boot the machine and wait for the guest agent to answer.
    pub fn start(&self) -> Result<()> {
        self.machine.start()?;
        Ok(())
    }

    pub fn wait_until_ready(&self) -> Result<()> {
        self.machine.wait_until_ready()?;
        Ok(())
    }

    pub fn wait_until_ready_with(&self, options: ReadyOptions) -> Result<()> {
        self.machine.wait_until_ready_with(options)?;
        Ok(())
    }

    /// Run a command in the guest and wait for it.
    pub fn exec<I, S>(&self, command: I) -> Result<ExecResult>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(self.machine.exec(command)?)
    }

    /// Run a command with explicit environment, directory or timeout.
    pub fn exec_with<I, S>(&self, command: I, options: ExecOptions) -> Result<ExecResult>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(self.machine.exec_with(command, options)?)
    }

    /// Run a command and read its output as it arrives.
    ///
    /// The stream is output-only: smolmachines exposes no stdin or PTY, so an
    /// interactive session needs a shell reachable over a port (see vmctl).
    pub fn exec_stream<I, S>(&self, command: I, options: ExecOptions) -> Result<ExecStream>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(self.machine.exec_stream(command, options)?)
    }

    /// Write a file into the running guest.
    pub fn write_file(&self, path: &str, data: impl Into<Vec<u8>>) -> Result<()> {
        self.machine.write_file(path, data)?;
        Ok(())
    }

    /// Read a file out of the running guest.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        Ok(self.machine.read_file(path)?)
    }

    /// The host port forwarding to a published guest port.
    pub fn host_port(&self, guest_port: u16) -> Result<Option<u16>> {
        Ok(self.machine.host_port(guest_port)?)
    }

    /// Shut the machine down, keeping its disks.
    pub fn stop(&self) -> Result<()> {
        self.machine.stop()?;
        Ok(())
    }

    /// Stop the machine and remove its storage. Not reversible.
    pub fn delete(&self) -> Result<()> {
        self.machine.delete()?;
        Ok(())
    }

    /// Copy guest-local staged mounts back to their host sources.
    pub fn sync(&self) -> Result<()> {
        self.machine.sync()?;
        Ok(())
    }

    /// Create a stopped machine from a portable checkpoint on disk.
    pub fn restore_checkpoint(name: impl Into<String>, artifact: impl AsRef<Path>) -> Result<Self> {
        ensure_runtime()?;
        let machine = Machine::restore_checkpoint(name, artifact)?;
        Ok(Self { machine })
    }

    /// The underlying engine handle, for anything this wrapper does not expose.
    pub fn inner(&self) -> &Machine {
        &self.machine
    }
}