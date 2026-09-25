use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use vm::{Error, ExecEvent, ExecOptions, ExecResult, Port, Vm};

/// Host port forwarded to the guest's sshd by `vmctl up`.
const SSH_HOST_PORT: u16 = 2222;
const SSH_GUEST_PORT: u16 = 22;

/// Set from `-d`/`--debug`; gates all diagnostic output.
static DEBUG: AtomicBool = AtomicBool::new(false);

fn debug_enabled() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

macro_rules! debug {
    ($($arg:tt)*) => {
        if $crate::debug_enabled() {
            eprintln!("[debug] {}", format_args!($($arg)*));
        }
    };
}

fn usage() -> ! {
    eprintln!("usage: vmctl [-d|--debug] <command> [name] [-- <command>...]");
    eprintln!();
    eprintln!("flags:");
    eprintln!("  -d, --debug  print diagnostic detail (setup steps, ssh args, timings)");
    eprintln!();
    eprintln!("commands:");
    eprintln!("  up [name]                 boot a debian:stable-slim machine (default name: alpine)");
    eprintln!("  stop [name]               stop the machine, keeping its disks");
    eprintln!("  delete [name]             stop the machine and remove its storage");
    eprintln!("  exec <name> -- <command>  run a command, streaming its output");
    eprintln!("  exec <name> -it -- [cmd]  run a command in a PTY via ssh (default: /bin/sh)");
    std::process::exit(2)
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();

    // Pull out global debug flags wherever they appear before the command.
    let mut args: Vec<String> = Vec::with_capacity(raw.len());
    for arg in raw {
        if arg == "-d" || arg == "--debug" {
            DEBUG.store(true, Ordering::Relaxed);
        } else {
            args.push(arg);
        }
    }

    let mut args = args.into_iter();
    let command = args.next().unwrap_or_default();
    let rest: Vec<String> = args.collect();

    debug!("argv: {command} {:?}", rest);

    let result = match command.as_str() {
        "up" => up(rest.first().map(String::as_str).unwrap_or("alpine")),
        "stop" => Vm::inspect(&arg_name(&rest)).and_then(|vm| vm.stop()).map(|()| 0),
        "delete" => Vm::inspect(&arg_name(&rest)).and_then(|vm| vm.delete()).map(|()| 0),
        "exec" => match parse_exec(&rest) {
            Some((name, tty, guest_command)) => {
                if tty {
                    exec_tty(&name, &guest_command)
                } else {
                    exec_streamed(&name, &guest_command)
                }
            }
            None => usage(),
        },
        "" => usage(),
        other => {
            eprintln!("unknown command: {other}");
            usage()
        }
    };

    match result {
        Ok(code) => {
            debug!("exit code {code}");
            ExitCode::from(code)
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn arg_name(rest: &[String]) -> String {
    rest.first().cloned().unwrap_or_else(|| "alpine".to_string())
}

/// `exec [name] [-it] [-- cmd...]` -> (name, tty, command)
fn parse_exec(rest: &[String]) -> Option<(String, bool, Vec<String>)> {
    let mut name: Option<String> = None;
    let mut tty = false;
    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i].as_str();
        if arg == "--" {
            i += 1;
            break;
        }
        if arg.starts_with('-') && arg.len() > 1 {
            let flags = arg.trim_start_matches('-');
            tty |= !flags.is_empty() && flags.chars().all(|c| c == 'i' || c == 't');
            i += 1;
            continue;
        }
        if name.is_none() {
            name = Some(rest[i].clone());
        }
        i += 1;
    }
    let command: Vec<String> = rest[i..].to_vec();
    let name = name?;
    if !tty && command.is_empty() {
        return None;
    }
    Some((name, tty, command))
}

/// Where we keep vmctl's ssh state inside the guest. Not /root/.ssh: the
/// engine masks that path, so it vanishes across stop/start.
const GUEST_SSH_DIR: &str = "/opt/smol-ssh";

/// Installs and prepares sshd. Waits out any apt/dpkg lock held by a
/// concurrent install (e.g. the one `vmctl up` runs at boot).
const SSH_INSTALL_SCRIPT: &str = "i=0; while ! (apt-get update -qq && \
     apt-get install -y -qq --no-install-recommends openssh-server); do \
     i=$((i+1)); [ $i -ge 30 ] && exit 1; echo 'apt busy, retrying...' >&2; sleep 2; done \
     && ssh-keygen -A && mkdir -p /run/sshd /opt/smol-ssh && chmod 700 /opt/smol-ssh";

fn up(name: &str) -> vm::Result<u8> {
    // Idempotent: reuse (and start) an existing machine; create when none
// exists. `create` failing with CONFLICT is the authoritative "exists"
// signal, since a broken machine still passes the non-starting attach.
let vm = match Vm::attach(name) {
        Ok(vm) => {
            debug!("reusing machine {name} (started if it was stopped)");
            vm
        }
        Err(start_err) => {
            debug!("attach {name} failed ({start_err}); trying create");
            let builder = Vm::builder(name.to_string())
                .image("debian:stable-slim")
                .cpus(1)
                .memory_mib(2048)
                .network(true)
                .overlay_gib(1)
                .storage_gib(1)
                .persistent(true)
                .port(Port::new(SSH_HOST_PORT, SSH_GUEST_PORT));
            match Vm::spawn(builder) {
                Ok(vm) => vm,
                Err(create_err) => {
                    return Err(Error::Runtime(format!(
                        "machine {name} exists but could not be started or recreated; start \
                         error: {start_err}; create error: {create_err}\n\
                         recreate it with `vmctl delete {name} && vmctl up {name}`"
                    )));
                }
            }
        }
    };

    let key = ensure_key()?;
    setup_sshd(&vm, &key)?;

    let uname = vm.exec(["uname", "-a"])?;
    println!("{} is up (pid {:?})", vm.name(), vm.pid());
    println!("{}", uname.stdout_utf8().trim());
    println!("run `vmctl exec {name} -it -- /bin/sh` for a shell");
    Ok(0)
}

fn exec_streamed(name: &str, command: &[String]) -> vm::Result<u8> {
    debug!("attach {name} and stream {:?}", command);
    let vm = Vm::attach(name)?;
    let mut exit_code = -1i32;
    for event in vm.exec_stream(command, ExecOptions::new())? {
        match event {
            ExecEvent::Stdout(bytes) => {
                std::io::stdout().write_all(&bytes)?;
                std::io::stdout().flush()?;
            }
            ExecEvent::Stderr(bytes) => {
                std::io::stderr().write_all(&bytes)?;
                std::io::stderr().flush()?;
            }
            ExecEvent::Exit(code) => exit_code = code,
            ExecEvent::Error(message) => return Err(Error::Runtime(message)),
        }
    }
    Ok(exit_code.clamp(0, 255) as u8)
}

/// Interactive exec: smolmachines has no stdin/PTY in its exec API, so this
/// reaches the guest through sshd on a forwarded port and lets the host's
/// ssh allocate the PTY.
fn exec_tty(name: &str, command: &[String]) -> vm::Result<u8> {
    debug!("attach {name} for interactive exec");
    let vm = Vm::attach(name)?;
    debug!("published guest ports: {:?}", vm.inner().guest_ports());
    let key = ensure_key()?;
    setup_sshd(&vm, &key)?;

    // `Machine::connect` handles do not report published ports, but the engine
// binds the forward in the VM process, so vmctl uses the port it chose at
// `up` time and verifies it by probing.
let host_port = SSH_HOST_PORT;
    debug!("host forward for guest port {SSH_GUEST_PORT}: {host_port}");

    // Fail fast instead of hanging: the port forward only exists if the
    // machine was created with `vmctl up` after the ssh port was added.
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], host_port));
    debug!("probing {addr} ...");
    let probe = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3));
    if probe.is_err() {
        return Err(Error::Runtime(format!(
            "nothing answering on {addr}; the port forward is not live. Recreate the machine \
             with `vmctl delete {name} && vmctl up {name}`"
        )));
    }
    drop(probe);
    debug!("{addr} reachable");

    let mut ssh = Command::new("ssh");
    ssh.args([
        "-tt",
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        "ConnectTimeout=5",
    ])
    .arg("-p")
    .arg(host_port.to_string())
    .arg("-i")
    .arg(&key)
    .arg(format!("root@127.0.0.1"));
    if !command.is_empty() {
        ssh.args(command);
    }
    debug!("exec: ssh -tt -p {host_port} -i {} root@127.0.0.1 {:?}", key.display(), command);
    let status = ssh.status()?;
    debug!("ssh exited with {:?}", status.code());
    Ok(status.code().unwrap_or(1).clamp(0, 255) as u8)
}

/// Run a guest command with a timeout, logging it under `--debug`.
fn guest_exec(vm: &Vm, script: &str, timeout: Duration) -> vm::Result<ExecResult> {
    debug!("guest$ {script}");
    let started = std::time::Instant::now();
    let result = vm.exec_with(["sh", "-c", script], ExecOptions::new().timeout(timeout))?;
    debug!(
        "guest$ -> exit {} in {:?} (stderr {:?})",
        result.exit_code,
        started.elapsed(),
        result.stderr_utf8().trim()
    );
    Ok(result)
}

/// Run a named guest step with visible status on stderr, always.
///
/// The command's own stdout/stderr are forwarded live so a slow network step
/// (apk fetch) shows progress instead of looking like a hang. Prints
/// `-> <label>` then `<label> ... (1.2s)` or `FAILED`.
fn guest_step(vm: &Vm, label: &str, script: &str, timeout: Duration) -> vm::Result<ExecResult> {
    debug!("guest$ {script}");
    eprintln!("  -> {label}");
    let started = std::time::Instant::now();
    let mut exit_code = -1i32;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut streamed = false;
    for event in vm.exec_stream(["sh", "-c", script], ExecOptions::new().timeout(timeout))? {
        match event {
            ExecEvent::Stdout(bytes) => {
                if debug_enabled() {
                    std::io::stderr().write_all(&bytes)?;
                    std::io::stderr().flush()?;
                    streamed = true;
                }
                stdout.extend(bytes);
            }
            ExecEvent::Stderr(bytes) => {
                std::io::stderr().write_all(&bytes)?;
                std::io::stderr().flush()?;
                streamed = true;
                stderr.extend(bytes);
            }
            ExecEvent::Exit(code) => exit_code = code,
            ExecEvent::Error(message) => return Err(Error::Runtime(message)),
        }
    }
    let elapsed = started.elapsed();
    if streamed {
        eprintln!();
    }
    eprintln!(
        "  {} {label} ({elapsed:?})",
        if exit_code == 0 { "OK" } else { "FAILED" }
    );
    debug!("guest$ -> exit {exit_code} in {elapsed:?}");
    Ok(ExecResult {
        exit_code,
        stdout,
        stderr,
    })
}

/// Generate (once) the ed25519 keypair vmctl uses to ssh into guests.
fn ensure_key() -> vm::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::Runtime("no HOME to store a vmctl key in".to_string()))?;
    let dir = home.join(".noema/vm");
    std::fs::create_dir_all(&dir)?;
    let key = dir.join("id_ed25519");
    if !key.is_file() {
        debug!("generating keypair at {}", key.display());
        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-q", "-f"])
            .arg(&key)
            .status()?;
        if !status.success() {
            return Err(Error::Runtime("ssh-keygen failed".to_string()));
        }
    } else {
        debug!("using existing keypair at {}", key.display());
    }
    Ok(key)
}

/// Make sure sshd is installed and running in the guest, with our key in
/// authorized_keys. Each step is skipped when already satisfied.
fn setup_sshd(vm: &Vm, key: &Path) -> vm::Result<()> {
    let present = guest_exec(
        vm,
        "test -x /usr/sbin/sshd && echo yes || echo no",
        Duration::from_secs(10),
    )?;
    if present.stdout_utf8().trim() != "yes" {
        debug!("openssh missing in guest, installing");
        let install = guest_step(vm, "install openssh", SSH_INSTALL_SCRIPT, Duration::from_secs(300))?;
        if !install.success() {
            return Err(Error::Runtime(format!(
                "installing sshd in the guest failed: {}",
                install.stderr_utf8().trim()
            )));
        }
    }

    let public_key = std::fs::read(key.with_extension("pub"))?;
    debug!("writing authorized_keys ({} bytes)", public_key.len());
    vm.write_file(&format!("{GUEST_SSH_DIR}/authorized_keys"), public_key)?;
    guest_exec(
        vm,
        &format!("chmod 700 {GUEST_SSH_DIR} && chmod 600 {GUEST_SSH_DIR}/authorized_keys"),
        Duration::from_secs(10),
    )?;

    let running = guest_exec(
        vm,
        "pidof sshd >/dev/null 2>&1 && echo yes || echo no",
        Duration::from_secs(10),
    )?;
    if running.stdout_utf8().trim() == "yes" {
        debug!("sshd already running in guest");
        return Ok(());
    }

    // /run is tmpfs in the guest, so /run/sshd must exist at every start.
    // AuthorizedKeysFile points at the persistent dir because the engine
    // masks /root/.ssh across stop/start.
    let start_script = format!(
        "mkdir -p /run/sshd && /usr/sbin/sshd -E /tmp/sshd.log \
         -o AuthorizedKeysFile={GUEST_SSH_DIR}/authorized_keys -o StrictModes=no"
    );
    let start = guest_step(vm, "start sshd", &start_script, Duration::from_secs(20))?;
    if !start.success() {
        return Err(Error::Runtime(format!(
            "starting sshd failed: {}",
            start.stderr_utf8().trim()
        )));
    }

    let running = guest_exec(
        vm,
        "pidof sshd >/dev/null 2>&1 && echo yes || echo no",
        Duration::from_secs(10),
    )?;
    if running.stdout_utf8().trim() != "yes" {
        let log = vm
            .read_file("/tmp/sshd.log")
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
            .unwrap_or_default();
        return Err(Error::Runtime(format!(
            "sshd is not running after start; guest log: {log}"
        )));
    }
    debug!("sshd running in guest");
    Ok(())
}