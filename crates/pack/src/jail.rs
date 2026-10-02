//! Jail policy and jailed execution: the only sanctioned way agent
//! code runs on the host.
//!
//! Everything executes through `ai-jail` (`ai-jail -- program …`),
//! whose defaults are already the right ones — no network, no GPU,
//! no display, no agent state, private home, minimal environment —
//! and where writes outside mounted folders are denied. A
//! [`JailPolicy`] therefore states only the narrow exceptions a
//! given agent role legitimately needs, and [`JailedCommand`]
//! assembles them into the flag list before the `--` separator.
//!
//! Two knobs exist because two roles have real needs (`network`,
//! `agent_state`); nothing else is exposed deliberately — no role in
//! this system has a legitimate use for GPU, display, host shared
//! memory, terminal passthrough, full-environment inheritance, or
//! host-home access, and an explicit knob is the first step toward
//! someone flipping it.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Cap on combined output returned to an agent.
pub const RUN_OUTPUT_CAP: usize = 20_000;

/// What one jailed execution may touch. The fields are the policy
/// surface; everything else stays at the ai-jail defaults (off).
#[derive(Debug, Clone, Default)]
pub struct JailPolicy {
    /// Directories mounted read-write — the only writable places.
    pub rw_dirs: Vec<PathBuf>,

    /// Directories mounted read-only in addition to the base mount.
    pub ro_dirs: Vec<PathBuf>,

    /// Unrestricted network. Exfiltrates anything the command can
    /// read; no current role needs it.
    pub network: bool,

    /// Mount the invoking process's credential state. Exposes those
    /// credentials to everything in the jail; no current role needs
    /// it.
    pub agent_state: bool,
}

impl JailPolicy {
    /// The data-preparation agent's policy: the pack directory
    /// writable (it holds the agent's artifacts), the raw data
    /// readable and never writable, nothing else.
    pub fn prepare(pack_dir: &Path, data_dir: &Path) -> Self {
        Self {
            rw_dirs: vec![pack_dir.to_path_buf()],
            ro_dirs: vec![data_dir.to_path_buf()],
            ..Self::default()
        }
    }

    /// The ai-jail flags carrying this policy, in order, before the
    /// `--` program separator. Capability flags are passed in both
    /// states explicitly so the assembled command line is a complete
    /// record of the policy — a default flipped in ai-jail itself
    /// cannot silently widen a jail.
    pub(crate) fn jail_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        for dir in &self.rw_dirs {
            args.push("--rw-map".to_string());
            args.push(dir.display().to_string());
        }

        for dir in &self.ro_dirs {
            args.push("--map".to_string());
            args.push(dir.display().to_string());
        }

        args.push(
            if self.network { "--network" } else { "--no-network" }.to_string(),
        );

        args.push(
            if self.agent_state { "--agent-state" } else { "--no-agent-state" }.to_string(),
        );

        args
    }
}

/// What one jailed execution produced: combined output (capped),
/// the exit code, and whether the timeout fired.
pub struct JailedOutput {
    pub text: String,
    pub exit: Option<i32>,
    pub timed_out: bool,
}

/// Read a pipe into a capped shared buffer on its own thread, so a
/// chatty jailed command can never deadlock the timeout poll below.
fn drain_pipe<R: Read + Send + 'static>(
    pipe: R,
    sink: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
) -> std::thread::JoinHandle<usize> {
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(pipe);
        let mut total = 0usize;
        let mut chunk = [0u8; 8192];

        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut sink = sink.lock().expect("output sink");

                    if sink.len() < RUN_OUTPUT_CAP {
                        let take = n.min(RUN_OUTPUT_CAP - sink.len());
                        sink.extend_from_slice(&chunk[..take]);
                    }

                    total += n;
                }
            }
        }

        total
    })
}

/// One jailed execution: `ai-jail <policy flags> -- program args…`
/// in `cwd`. Environment variables are exported into the shell
/// string itself — the jail passes only a minimal environment, so
/// this is the one channel that reliably reaches the command.
pub struct JailedCommand {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
    envs: Vec<(String, String)>,
    policy: JailPolicy,
}

impl JailedCommand {
    pub fn new(program: &str, cwd: &Path, policy: JailPolicy) -> Self {
        Self {
            program: program.to_string(),
            args: Vec::new(),
            cwd: cwd.to_path_buf(),
            envs: Vec::new(),
            policy,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.envs.push((key.to_string(), value.to_string()));
        self
    }

    fn export(assignment: &str, value: &str) -> String {
        // Single-quote the value so paths with spaces or quotes survive.
        let quoted = value.replace('\'', "'\\''");

        format!("export {assignment}='{quoted}'; ")
    }

    pub fn run_shell(self, command: &str, timeout: std::time::Duration) -> Result<JailedOutput> {
        let prefix: String = self
            .envs
            .iter()
            .map(|(key, value)| Self::export(key, value))
            .collect();

        let mut child = std::process::Command::new("ai-jail")
            .current_dir(&self.cwd)
            .args(self.policy.jail_args())
            .arg("--")
            .arg(&self.program)
            .args(&self.args)
            .arg(format!("{prefix}{command}"))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("spawning `ai-jail` — is it on PATH?")?;

        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        let stdout = child
            .stdout
            .take()
            .map(|pipe| drain_pipe(pipe, sink.clone()))
            .into_iter()
            .collect::<Vec<_>>();

        let stderr = child
            .stderr
            .take()
            .map(|pipe| drain_pipe(pipe, sink.clone()))
            .into_iter()
            .collect::<Vec<_>>();

        let started = std::time::Instant::now();

        let (exit, timed_out) = loop {
            match child.try_wait()? {
                Some(status) => break (status.code(), false),
                None if started.elapsed() >= timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break (None, true);
                }
                None => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        };

        for reader in stdout.into_iter().chain(stderr) {
            let _ = reader.join();
        }

        let captured = sink.lock().expect("output sink").clone();

        let mut text = String::from_utf8_lossy(&captured).to_string();

        if text.len() >= RUN_OUTPUT_CAP {
            text.push_str("\n… output truncated …");
        }

        Ok(JailedOutput {
            text,
            exit,
            timed_out,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_assembles_explicit_mounts_and_caps() {
        let policy = JailPolicy::prepare(Path::new("/packs/toy"), Path::new("/data/raw"));

        let args = policy.jail_args();
        let text = args.join(" ");

        assert_eq!(
            args,
            vec![
                "--rw-map",
                "/packs/toy",
                "--map",
                "/data/raw",
                "--no-network",
                "--no-agent-state",
            ],
        );

        // Capability flags are passed in both states: a widened
        // ai-jail default cannot silently leak into a jail.
        assert!(text.contains("--no-network"));

        let open = JailPolicy {
            network: true,
            agent_state: true,
            ..JailPolicy::default()
        };

        assert_eq!(open.jail_args(), vec!["--network", "--agent-state"]);
    }

    #[test]
    fn jailed_commands_run_and_time_out() {
        let policy = JailPolicy {
            rw_dirs: vec![PathBuf::from("/tmp")],
            ..JailPolicy::default()
        };

        let outcome = match JailedCommand::new("sh", Path::new("/tmp"), policy)
            .arg("-c")
            .env("DATA_DIR", "/nowhere")
            .run_shell("echo jailed-$DATA_DIR", std::time::Duration::from_secs(30))
        {
            Ok(outcome) => outcome,
            Err(error) => {
                let message = format!("{error:#}");

                // A machine without ai-jail is an environment gap,
                // not a jail defect (same concession as the plugin
                // load test).
                assert!(
                    message.contains("ai-jail"),
                    "jailed run failed: {message}",
                );

                eprintln!("skipping jail test: {message}");

                return;
            }
        };

        assert_eq!(outcome.exit, Some(0));
        assert!(outcome.text.contains("jailed-/nowhere"));

        let timeout = JailedCommand::new("sh", Path::new("/tmp"), JailPolicy::default())
            .arg("-c")
            .run_shell("sleep 30", std::time::Duration::from_millis(500))
            .expect("ai-jail present");

        assert!(timeout.timed_out);
    }
}