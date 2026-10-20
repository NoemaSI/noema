//! Generic Rumoca/Modelica execution, shared by every environment.
//!
//! Two worlds build on this crate:
//!
//! - the warehouse (`warehouse_core`), which compiles one fixed plant;
//! - the protein-binding benchmark (`binding`), which compiles arbitrary
//!   candidate models on demand, exactly like the plugin loader compiles
//!   arbitrary plugin sources.
//!
//! The contract with Rumoca was verified against the generated CasADi
//! artifacts, not assumed: `rumoca compile --target casadi-mx <file>.mo`
//! emits `<project>/<Model>/<Model>_casadi_mx.py`, whose
//! `create_model()` returns a dict with `state_names`, `input_names`,
//! `param_names`, `algebraic_names`, start vectors `x0/u0/p0`, and
//! `build_integrator(times)` accepting a scalar step *or* an output-time
//! grid. The integrator consumes `p = concat(parameters, inputs)` and
//! returns `xf` plus `zf`; on the augmented path `zf` stacks the state
//! derivatives (first `n_x` rows) above the continuous algebraics in
//! `algebraic_names` order.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub enum ModelicaError {
    ModelNotFound(PathBuf),
    Io(std::io::Error),
    SpawnFailed {
        program: String,
        current_dir: PathBuf,
        source: std::io::Error,
    },
    ProcessFailed {
        command: String,
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    ArtifactNotFound(PathBuf),
    InvalidMetadata(String),
}

impl std::fmt::Display for ModelicaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelNotFound(path) => {
                write!(formatter, "Modelica model not found: {}", path.display())
            }
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::SpawnFailed {
                program,
                current_dir,
                source,
            } => {
                let path = std::env::var("PATH").unwrap_or_else(|_| "<unset>".to_string());

                write!(
                    formatter,
                    "could not spawn `{program}` (cwd {}): {source}\n\
                     hint: is `{program}` installed and on PATH?\n\
                     PATH={path}",
                    current_dir.display(),
                )
            }
            Self::ProcessFailed {
                command,
                code,
                stdout,
                stderr,
            } => write!(
                formatter,
                "`{command}` failed (exit {code:?})\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
            ),
            Self::ArtifactNotFound(path) => {
                write!(formatter, "compiled artifact not found: {}", path.display())
            }
            Self::InvalidMetadata(message) => {
                write!(formatter, "invalid artifact metadata: {message}")
            }
        }
    }
}

impl std::error::Error for ModelicaError {}

impl From<std::io::Error> for ModelicaError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ModelicaError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidMetadata(error.to_string())
    }
}

/// A Modelica model compiled to the CasADi MX target, ready to simulate.
#[derive(Debug, Clone)]
pub struct CompiledModel {
    pub source: PathBuf,
    pub target: String,
    pub artifact: PathBuf,
    pub dynamics: ModelDynamics,
}

/// Structure metadata of a compiled model, read from the artifact.
#[derive(Debug, Clone)]
pub struct ModelDynamics {
    pub states: Vec<String>,
    pub inputs: Vec<String>,
    pub parameters: Vec<String>,
    pub algebraics: Vec<String>,
    pub dynamics: String,
    pub f_x: String,
    pub ode: String,
    pub x0: Vec<f64>,
    pub u0: Vec<f64>,
    pub p0: Vec<f64>,
}

/// Repository root: the directory holding `pixi.toml`, used as the
/// working directory for every `pixi run python` invocation.
///
/// Defaults to the manifest's parent directory (the repository
/// checkout as it was at compile time) and can be overridden with
/// `RWSI_ROOT_DIR`: the compile-time path goes stale when the
/// checkout moves, and already-built binaries do not re-expand it.
pub fn workspace_root() -> PathBuf {
    if let Ok(root) = std::env::var("RWSI_ROOT_DIR") {
        if !root.is_empty() {
            return PathBuf::from(root);
        }
    }

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));

    manifest
        .parent()
        .map_or_else(|| manifest.to_path_buf(), |root| root.to_path_buf())
}

/// Run a subprocess, mapping a failed spawn (binary missing, not
/// executable, ...) to a [`ModelicaError::SpawnFailed`] that names the
/// program, the working directory it was launched from, and the
/// current `PATH`.
fn spawn(command: &mut Command) -> Result<std::process::Output, ModelicaError> {
    let program = command.get_program().to_string_lossy().into_owned();

    let current_dir = command
        .get_current_dir()
        .map_or_else(|| std::env::current_dir().unwrap_or_default(), Path::to_path_buf);

    command.output().map_err(|source| ModelicaError::SpawnFailed {
        program,
        current_dir,
        source,
    })
}

/// Compile `<project_dir>/<mo_file>` with Rumoca's CasADi MX target.
///
/// `model_name` is the Modelica model identifier (the artifact
/// directory is named after the model, not the file).
pub fn compile(
    project_dir: &Path,
    mo_file: &str,
    model_name: &str,
) -> Result<CompiledModel, ModelicaError> {
    if !project_dir.join(mo_file).exists() {
        return Err(ModelicaError::ModelNotFound(
            project_dir.join(mo_file),
        ));
    }

    let command = format!(
        "rumoca compile --target casadi-mx {mo_file}"
    );

    let mut process = noema_config::toolchain::command("rumoca");
    let output = spawn(
        process
            .current_dir(project_dir)
            .args(["compile", "--target", "casadi-mx", mo_file]),
    )?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if !output.status.success() {
        return Err(ModelicaError::ProcessFailed {
            command,
            code: output.status.code(),
            stdout,
            stderr,
        });
    }

    let artifact = project_dir
        .join(model_name)
        .join(format!("{model_name}_casadi_mx.py"));

    if !artifact.exists() {
        return Err(ModelicaError::ArtifactNotFound(artifact));
    }

    let source = project_dir.join(mo_file);
    let dynamics = inspect(&artifact)?;

    Ok(CompiledModel {
        source,
        target: "casadi-mx".to_string(),
        artifact,
        dynamics,
    })
}

/// Read structure metadata out of a generated CasADi artifact.
pub fn inspect(artifact: &Path) -> Result<ModelDynamics, ModelicaError> {
    let python = r#"
import json
import sys
import casadi as ca

sys.path.insert(0, sys.argv[1])

module_name = sys.argv[2]
module = __import__(module_name)

model = module.create_model()

# The explicit dynamics function only exists for pure ODEs; on the
# augmented DAE path the `ode` expression still references the
# augmented algebraic symbol, so fall back to the residual form.
try:
    dynamics = str(
        ca.Function(
            "dynamics",
            [model["x"], model["u"], model["p"]],
            [model["ode"]],
        )
    )
except Exception:
    dynamics = str(model["f_x"])

print(json.dumps({
    "state_names": model["state_names"],
    "input_names": model["input_names"],
    "param_names": model["param_names"],
    "algebraic_names": model["algebraic_names"],
    "f_x": str(model["f_x"]),
    "ode": str(model["ode"]),
    "x0": model["x0"].flatten().tolist(),
    "u0": model["u0"].flatten().tolist(),
    "p0": model["p0"].flatten().tolist(),
    "dynamics": dynamics,
}))
"#;

    let directory = artifact.parent().ok_or_else(|| {
        ModelicaError::InvalidMetadata(
            "CasADi artifact has no parent directory".to_string(),
        )
    })?;

    let module_name = artifact
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            ModelicaError::InvalidMetadata(
                "Invalid CasADi artifact filename".to_string(),
            )
        })?;

    let mut process = noema_config::toolchain::command("pixi");
    let output = spawn(
        process
            .current_dir(workspace_root())
            .arg("run")
            .arg("python")
            .arg("-c")
            .arg(python)
            .arg(directory)
            .arg(module_name),
    )?;

    if !output.status.success() {
        return Err(ModelicaError::ProcessFailed {
            command: "python <casadi artifact inspection>".to_string(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let metadata = String::from_utf8_lossy(&output.stdout);

    let value: serde_json::Value =
        serde_json::from_str(metadata.trim()).map_err(|error| {
            ModelicaError::InvalidMetadata(error.to_string())
        })?;

    Ok(ModelDynamics {
        states: names(&value, "state_names")?,
        inputs: names(&value, "input_names")?,
        parameters: names(&value, "param_names")?,
        algebraics: names(&value, "algebraic_names")?,
        dynamics: text(&value, "dynamics")?,
        f_x: text(&value, "f_x")?,
        ode: text(&value, "ode")?,
        x0: numbers(&value, "x0")?,
        u0: numbers(&value, "u0")?,
        p0: numbers(&value, "p0")?,
    })
}

/// A simulated trajectory sampled on the requested time grid.
///
/// `states[time][state]`; `algebraics[time][algebraic]` is empty for
/// pure-ODE models (no continuous algebraics).
#[derive(Debug, Clone)]
pub struct SimTrace {
    pub states: Vec<Vec<f64>>,
    pub algebraics: Vec<Vec<f64>>,
}

impl CompiledModel {
    /// Simulate on an absolute output-time grid starting at 0, holding
    /// `inputs` constant for the whole run.
    ///
    /// `parameters` overrides the artifact start values (the fitter's
    /// theta vector, in `dynamics.parameters` order).
    pub fn simulate(
        &self,
        parameters: &[f64],
        inputs: &[f64],
        times: &[f64],
    ) -> Result<SimTrace, ModelicaError> {
        self.simulate_protocol(parameters, inputs, None, times)
    }

    /// Simulate a two-phase protocol: `inputs` are held at
    /// their given values until `t_step`, then switched to zero and
    /// the integration continued from the state reached at `t_step`.
    ///
    /// This is the binding-washout special case of
    /// [`simulate_protocol_after`](Self::simulate_protocol_after)
    /// (post-step level zero); see there for the general semantics.
    /// `t_step = None` (or outside the time span) reproduces the
    /// constant-input [`simulate`](Self::simulate) behavior exactly.
    pub fn simulate_protocol(
        &self,
        parameters: &[f64],
        inputs: &[f64],
        t_step: Option<f64>,
        times: &[f64],
    ) -> Result<SimTrace, ModelicaError> {
        self.simulate_protocol_after(parameters, inputs, t_step, 0.0, times)
    }

    /// Simulate a two-phase piecewise-constant protocol: `inputs`
    /// are held at their given values until `t_step`, then switched
    /// to `after` (per input) and the integration continued from the
    /// state reached at `t_step`.
    ///
    /// The model itself stays time-invariant; only the driving
    /// signal carries the protocol. The caller (data ingestion)
    /// decides what the step means physically — e.g. a BLI tip
    /// moving from analyte (input = C) to buffer (after = 0).
    /// `t_step = None` (or outside the time span) reproduces the
    /// constant-input [`simulate`](Self::simulate) behavior exactly.
    pub fn simulate_protocol_after(
        &self,
        parameters: &[f64],
        inputs: &[f64],
        t_step: Option<f64>,
        after: f64,
        times: &[f64],
    ) -> Result<SimTrace, ModelicaError> {
        if self.dynamics.parameters.len() != parameters.len() {
            return Err(ModelicaError::InvalidMetadata(format!(
                "expected {} parameter values, got {}",
                self.dynamics.parameters.len(),
                parameters.len(),
            )));
        }

        if self.dynamics.inputs.len() != inputs.len() {
            return Err(ModelicaError::InvalidMetadata(format!(
                "expected {} input values, got {}",
                self.dynamics.inputs.len(),
                inputs.len(),
            )));
        }

        let python = r#"
import json
import sys
import numpy as np
import casadi as ca

sys.path.insert(0, sys.argv[1])

module = __import__(sys.argv[2])
model = module.create_model()

times = json.loads(sys.argv[3])
p = np.array(json.loads(sys.argv[4]), dtype=float).reshape((-1, 1))
u = np.array(json.loads(sys.argv[5]), dtype=float).reshape((-1, 1))

# Optional protocol step: inputs switch to `after` at t_step (see
# CompiledModel::simulate_protocol_after). Negative t_step means
# "no step"; `after` defaults to 0 (the legacy washout-to-zero).
t_step = float(sys.argv[6]) if len(sys.argv) > 6 else -1.0
after = float(sys.argv[7]) if len(sys.argv) > 7 else 0.0
u_after = np.full_like(u, after)


def integrate(t0, grid, u_values, x0, z0_override):
    """Integrate the model over `grid`, starting the state at t0.

    A grid starting at t0 = 0 uses the artifact's own integrator
    builder; a shifted start builds an equivalent one from the raw
    DAE with a nonzero t0 (build_integrator hardcodes t0 = 0).
    """
    if t0 == 0.0:
        integrator = model["build_integrator"](grid)
    else:
        dae, method = model["build_dae"]()
        integrator = ca.integrator("integrator", method, dae, t0, grid, {})

    # DAE models integrate in augmented form: the integrator's z0
    # input is [xdot; continuous algebraics] and only exists when
    # there is an algebraic block (pure ODEs run on CVODES with no
    # z0 input).
    arguments = {
        "x0": x0,
        "p": np.concatenate((p, u_values)),
    }

    z_size = int(np.prod(integrator.size_in(1))) if integrator.n_in() > 1 else 0

    if z_size > 0:
        if z0_override is not None:
            arguments["z0"] = z0_override
        else:
            z0 = np.zeros((z_size, 1))
            z0[model["n_x"]:] = np.asarray(model["z0"], dtype=float).reshape((-1, 1))
            arguments["z0"] = z0

    return integrator(**arguments)


def strip_algebraics(zf):
    # The augmented z carries xdot in its first n_x rows; the
    # algebraics are what remains.
    zf = np.asarray(zf)
    if zf.ndim == 2 and zf.shape[0] > model["n_x"]:
        zf = zf[model["n_x"]:]
    return zf.T.tolist() if zf.ndim == 2 and zf.shape[0] > 0 else []


segments = []

if t_step < 0.0 or not times or t_step >= times[-1]:
    # Constant input over the whole run (the legacy behavior).
    segments.append((0.0, times, u, model["x0"], None, len(times)))
elif t_step <= times[0]:
    # The washout precedes the record: pure dissociation.
    segments.append((0.0, times, u_after, model["x0"], None, len(times)))
else:
    # The boundary point t_step itself is not an observation time in
    # general; it is appended to segment A's grid so the handoff
    # state is reached exactly, and its output column is dropped from
    # the stitched trace so rows stay 1:1 with `times`.
    n_a = sum(1 for value in times if value <= t_step)
    grid_a = [value for value in times if value <= t_step]
    if grid_a[-1] < t_step:
        grid_a.append(t_step)
    grid_b = [value for value in times if value > t_step]

    result_a = integrate(0.0, grid_a, u, model["x0"], None)
    segments.append(("result", result_a, n_a))

    xf_a = np.asarray(result_a["xf"]).reshape((model["n_x"], -1))
    zf_a = result_a.get("zf")
    x0_b = xf_a[:, -1].reshape((-1, 1))
    z0_b = (
        np.asarray(zf_a).reshape((-1, np.asarray(zf_a).shape[-1]))[:, -1].reshape((-1, 1))
        if zf_a is not None
        else None
    )

    result_b = integrate(t_step, grid_b, u_after, x0_b, z0_b)
    segments.append(("result", result_b, len(grid_b)))

states = []
algebraics = []

for segment in segments:
    if segment[0] == "result":
        result, keep = segment[1], segment[2]
    else:
        result = integrate(*segment[:5])
        keep = segment[5]

    xf = np.asarray(result["xf"]).reshape((model["n_x"], -1))[:, :keep]
    states.extend(xf.T.tolist())

    zf = result.get("zf")
    if zf is not None:
        zf = np.asarray(zf)
        if zf.ndim == 2:
            zf = zf[:, :keep]
        algebraics.extend(strip_algebraics(zf))

print(json.dumps({"states": states, "algebraics": algebraics}))
"#;

        let directory = self.artifact.parent().ok_or_else(|| {
            ModelicaError::InvalidMetadata(
                "CasADi artifact has no parent directory".to_string(),
            )
        })?;

        let module_name =
            self.artifact.file_stem().and_then(|name| name.to_str()).ok_or_else(|| {
                ModelicaError::InvalidMetadata(
                    "Invalid CasADi artifact filename".to_string(),
                )
            })?;

        let mut command = noema_config::toolchain::command("pixi");

        command
            .current_dir(workspace_root())
            .arg("run")
            .arg("python")
            .arg("-c")
            .arg(python)
            .arg(directory)
            .arg(module_name)
            .arg(serde_json::to_string(times)?)
            .arg(serde_json::to_string(parameters)?)
            .arg(serde_json::to_string(inputs)?);

        if let Some(t_step) = t_step {
            command.arg(t_step.to_string());
            command.arg(after.to_string());
        }

        let output = spawn(&mut command)?;

        if !output.status.success() {
            return Err(ModelicaError::ProcessFailed {
                command: "python <casadi artifact simulation>".to_string(),
                code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                ModelicaError::InvalidMetadata(error.to_string())
            })?;

        let states = matrix(&value, "states")?;
        let algebraics = matrix(&value, "algebraics")?;

        Ok(SimTrace {
            states,
            algebraics,
        })
    }
}

/// One recorded drive trace for
/// [`CompiledModel::simulate_traces`]: piecewise-constant samples
/// (zero-order hold: `values[i]` held from `t[i]` until `t[i+1]`,
/// the last value held to the end).
#[derive(Debug, Clone, Copy)]
pub struct DriveSamples<'a> {
    /// Model input name (a `dynamics.inputs` entry).
    pub channel: &'a str,

    /// Sample times (non-decreasing, at least one sample).
    pub t: &'a [f64],

    /// Value held from each sample time onwards.
    pub values: &'a [f64],
}

impl CompiledModel {
    /// Simulate on an absolute output-time grid, replaying each
    /// model input's recorded drive trace: the drive is
    /// piecewise-constant between its samples, and the integration
    /// continues through every breakpoint with exact state handoff.
    ///
    /// This is the general replay of which
    /// [`simulate_protocol_after`](Self::simulate_protocol_after) is
    /// the single-step special case: a constant input is a
    /// one-sample trace, and a `[(0, C), (t_step, after)]` trace
    /// reproduces `simulate_protocol_after` exactly (same segment
    /// grids, same handoff). Every declared model input must carry
    /// exactly one drive; extra drives for undeclared channels are
    /// rejected.
    pub fn simulate_traces(
        &self,
        parameters: &[f64],
        drives: &[DriveSamples<'_>],
        times: &[f64],
    ) -> Result<SimTrace, ModelicaError> {
        if self.dynamics.parameters.len() != parameters.len() {
            return Err(ModelicaError::InvalidMetadata(format!(
                "expected {} parameter values, got {}",
                self.dynamics.parameters.len(),
                parameters.len(),
            )));
        }

        let mut drives_json = serde_json::Map::new();

        for drive in drives {
            if !self
                .dynamics
                .inputs
                .iter()
                .any(|name| name == drive.channel)
            {
                return Err(ModelicaError::InvalidMetadata(format!(
                    "drive for undeclared input channel {:?}",
                    drive.channel,
                )));
            }

            if drive.t.is_empty() || drive.t.len() != drive.values.len() {
                return Err(ModelicaError::InvalidMetadata(format!(
                    "drive for {:?} needs matching, non-empty samples",
                    drive.channel,
                )));
            }

            drives_json.insert(
                drive.channel.to_string(),
                serde_json::json!([drive.t, drive.values]),
            );
        }

        for name in &self.dynamics.inputs {
            if !drives_json.contains_key(name.as_str()) {
                return Err(ModelicaError::InvalidMetadata(format!(
                    "no drive recorded for model input {:?}",
                    name,
                )));
            }
        }

        let python = r#"
import json
import sys
import numpy as np
import casadi as ca

sys.path.insert(0, sys.argv[1])

module = __import__(sys.argv[2])
model = module.create_model()

times = json.loads(sys.argv[3])
p = np.array(json.loads(sys.argv[4]), dtype=float).reshape((-1, 1))
drives = json.loads(sys.argv[5])

# Replay each model input's recorded drive trace with zero-order
# hold (CompiledModel::simulate_traces). Segment boundaries are the
# sample times where any channel's held value changes; between them
# the inputs are constant parameters, exactly as the single-step
# protocol path integrates them.
channels = model["input_names"]

samples = {
    name: (
        np.asarray(drives[name][0], dtype=float),
        np.asarray(drives[name][1], dtype=float),
    )
    for name in channels
}


def values_at(x):
    """Every channel's held value at time x (last sample at or
    before x; the first sample before the record start)."""
    held = []

    for name in channels:
        t_s, v_s = samples[name]
        index = int(np.searchsorted(t_s, x, side="right")) - 1
        held.append(v_s[index if index >= 0 else 0])

    return np.array(held, dtype=float).reshape((-1, 1))


breakpoints = set()

for name in channels:
    t_s, v_s = samples[name]

    for i in range(1, len(t_s)):
        if v_s[i] != v_s[i - 1]:
            breakpoints.add(float(t_s[i]))

b_used = sorted(
    b for b in breakpoints if times and times[0] < b < times[-1]
)


def integrate(t0, grid, u_values, x0, z0_override):
    """Integrate the model over `grid`, starting the state at t0.

    A grid starting at t0 = 0 uses the artifact's own integrator
    builder; a shifted start builds an equivalent one from the raw
    DAE with a nonzero t0 (build_integrator hardcodes t0 = 0).
    """
    if t0 == 0.0:
        integrator = model["build_integrator"](grid)
    else:
        dae, method = model["build_dae"]()
        integrator = ca.integrator("integrator", method, dae, t0, grid, {})

    # DAE models integrate in augmented form: the integrator's z0
    # input is [xdot; continuous algebraics] and only exists when
    # there is an algebraic block (pure ODEs run on CVODES with no
    # z0 input).
    arguments = {
        "x0": x0,
        "p": np.concatenate((p, u_values)),
    }

    z_size = int(np.prod(integrator.size_in(1))) if integrator.n_in() > 1 else 0

    if z_size > 0:
        if z0_override is not None:
            arguments["z0"] = z0_override
        else:
            z0 = np.zeros((z_size, 1))
            z0[model["n_x"]:] = np.asarray(model["z0"], dtype=float).reshape((-1, 1))
            arguments["z0"] = z0

    return integrator(**arguments)


def strip_algebraics(zf):
    # The augmented z carries xdot in its first n_x rows; the
    # algebraics are what remains.
    zf = np.asarray(zf)
    if zf.ndim == 2 and zf.shape[0] > model["n_x"]:
        zf = zf[model["n_x"]:]
    return zf.T.tolist() if zf.ndim == 2 and zf.shape[0] > 0 else []


segments = []

if not times:
    pass
elif not b_used:
    # Constant drive over the whole run (the legacy behavior).
    result = integrate(times[0], times, values_at(times[0]), model["x0"], None)
    segments.append((result, len(times)))
else:
    # Boundary points are not observation times in general; each
    # segment's grid gets its right boundary appended so the handoff
    # state is reached exactly, and the appended column is dropped
    # from the stitched trace so rows stay 1:1 with `times`.
    boundaries = [times[0]] + b_used + [times[-1]]

    x0 = model["x0"]
    z0 = None

    for k in range(len(boundaries) - 1):
        start, end = boundaries[k], boundaries[k + 1]

        if k == 0:
            grid = [value for value in times if value <= end]
        else:
            grid = [value for value in times if start < value <= end]

        keep = len(grid)

        if k + 2 < len(boundaries) and (not grid or grid[-1] < end):
            grid.append(end)

        if not grid:
            # Pure handoff segment: no observations inside, integrate
            # just to the boundary.
            grid = [end]

        result = integrate(start, grid, values_at(start), x0, z0)
        segments.append((result, keep))

        x0 = np.asarray(result["xf"]).reshape((model["n_x"], -1))[:, -1].reshape((-1, 1))

        zf = result.get("zf")

        z0 = (
            np.asarray(zf).reshape((-1, np.asarray(zf).shape[-1]))[:, -1].reshape((-1, 1))
            if zf is not None
            else None
        )

states = []
algebraics = []

for result, keep in segments:
    xf = np.asarray(result["xf"]).reshape((model["n_x"], -1))[:, :keep]
    states.extend(xf.T.tolist())

    zf = result.get("zf")
    if zf is not None:
        zf = np.asarray(zf)
        if zf.ndim == 2:
            zf = zf[:, :keep]
        algebraics.extend(strip_algebraics(zf))

print(json.dumps({"states": states, "algebraics": algebraics}))
"#;

        let directory = self.artifact.parent().ok_or_else(|| {
            ModelicaError::InvalidMetadata(
                "CasADi artifact has no parent directory".to_string(),
            )
        })?;

        let module_name = self
            .artifact
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                ModelicaError::InvalidMetadata(
                    "Invalid CasADi artifact filename".to_string(),
                )
            })?;

        let mut command = noema_config::toolchain::command("pixi");

        command
            .current_dir(workspace_root())
            .arg("run")
            .arg("python")
            .arg("-c")
            .arg(python)
            .arg(directory)
            .arg(module_name)
            .arg(serde_json::to_string(times)?)
            .arg(serde_json::to_string(parameters)?)
            .arg(serde_json::to_string(&serde_json::Value::Object(drives_json))?);

        let output = spawn(&mut command)?;

        if !output.status.success() {
            return Err(ModelicaError::ProcessFailed {
                command: "python <casadi artifact trace simulation>".to_string(),
                code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                ModelicaError::InvalidMetadata(error.to_string())
            })?;

        let states = matrix(&value, "states")?;
        let algebraics = matrix(&value, "algebraics")?;

        Ok(SimTrace {
            states,
            algebraics,
        })
    }
}

impl SimTrace {
    /// Extract one named variable (state or continuous algebraic) as a
    /// time series.
    ///
    /// `algebraics` holds only the continuous algebraics (the augmented
    /// `zf` state-derivative rows are stripped during simulation), so no
    /// state offset applies here.
    pub fn trace_of(
        &self,
        dynamics: &ModelDynamics,
        name: &str,
    ) -> Option<Vec<f64>> {
        if let Some(index) =
            dynamics.states.iter().position(|state| state == name)
        {
            return Some(
                self.states
                    .iter()
                    .map(|sample| sample.get(index).copied().unwrap_or(0.0))
                    .collect(),
            );
        }

        let index =
            dynamics.algebraics.iter().position(|algebraic| algebraic == name)?;

        Some(
            self.algebraics
                .iter()
                .map(|sample| sample.get(index).copied().unwrap_or(0.0))
                .collect(),
        )
    }
}

fn names(
    value: &serde_json::Value,
    field: &str,
) -> Result<Vec<String>, ModelicaError> {
    let values = value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            ModelicaError::InvalidMetadata(format!(
                "CasADi artifact did not provide `{field}` metadata"
            ))
        })?;

    values
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                ModelicaError::InvalidMetadata(format!(
                    "Invalid `{field}` variable metadata: {value}"
                ))
            })
        })
        .collect()
}

fn text(
    value: &serde_json::Value,
    field: &str,
) -> Result<String, ModelicaError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            ModelicaError::InvalidMetadata(format!(
                "CasADi artifact did not provide `{field}` metadata"
            ))
        })
}

fn numbers(
    value: &serde_json::Value,
    field: &str,
) -> Result<Vec<f64>, ModelicaError> {
    let values = value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            ModelicaError::InvalidMetadata(format!(
                "CasADi artifact did not provide `{field}` metadata"
            ))
        })?;

    values
        .iter()
        .map(|value| {
            value.as_f64().ok_or_else(|| {
                ModelicaError::InvalidMetadata(format!(
                    "Invalid `{field}` numeric metadata: {value}"
                ))
            })
        })
        .collect()
}

fn matrix(
    value: &serde_json::Value,
    field: &str,
) -> Result<Vec<Vec<f64>>, ModelicaError> {
    let rows = value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            ModelicaError::InvalidMetadata(format!(
                "CasADi artifact did not provide `{field}` metadata"
            ))
        })?;

    rows
        .iter()
        .map(|row| {
            row.as_array()
                .map(|values| {
                    values
                        .iter()
                        .map(|value| value.as_f64().unwrap_or(f64::NAN))
                        .collect()
                })
                .ok_or_else(|| {
                    ModelicaError::InvalidMetadata(format!(
                        "Invalid `{field}` row: {row}"
                    ))
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_failure_names_the_program() {
        let error = ModelicaError::SpawnFailed {
            program: "rumoca".to_string(),
            current_dir: PathBuf::from("/tmp"),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };

        let text = error.to_string();

        assert!(text.contains("could not spawn `rumoca`"), "{text}");
        assert!(text.contains("cwd /tmp"), "{text}");
        assert!(text.contains("is `rumoca` installed and on PATH?"), "{text}");
        assert!(text.contains("PATH="), "{text}");
    }

    #[test]
    fn compiles_and_simulates_relaxation_candidate() {
        let project = workspace_root().join("assets/candidates");

        let model = compile(&project, "Relaxation.mo", "Relaxation")
            .expect("Relaxation candidate should compile");

        assert_eq!(model.dynamics.states, vec!["x"]);
        assert_eq!(model.dynamics.inputs, vec!["concentration"]);
        assert_eq!(model.dynamics.parameters, vec!["p1", "p2"]);
        assert_eq!(model.dynamics.algebraics, vec!["signal"]);

        // p1 = 2, p2 = 0.5, C = 1: x(t) = 4 (1 - e^{-0.5 t}).
        let times = vec![0.0, 1.0, 2.0, 10.0];

        let trace = model
            .simulate(&[2.0, 0.5], &[1.0], &times)
            .expect("simulation should succeed");

        let signal = trace
            .trace_of(&model.dynamics, "signal")
            .expect("signal is a named algebraic");

        assert_eq!(signal.len(), times.len());
        assert!((signal[0] - 0.0).abs() < 1e-9);
        assert!((signal[1] - 4.0 * (1.0 - (-0.5f64).exp())).abs() < 1e-6);
        assert!((signal[2] - 4.0 * (1.0 - (-1.0f64).exp())).abs() < 1e-6);
        assert!((signal[3] - 4.0 * (1.0 - (-5.0f64).exp())).abs() < 1e-3);
    }

    #[test]
    fn protocol_stitches_association_and_dissociation() {
        let project = workspace_root().join("assets/candidates");

        let model = compile(&project, "Relaxation.mo", "Relaxation")
            .expect("Relaxation candidate should compile");

        // p1 = 2, p2 = 0.5, C = 1: rise x(t) = 4 (1 - e^{-0.5 t})
        // until the washout at t_step, then x decays exponentially.
        let times = vec![0.0, 1.0, 2.0, 3.0, 10.0];

        let washout = 4.0 * (1.0 - (-1.0f64).exp());

        let trace = model
            .simulate_protocol(&[2.0, 0.5], &[1.0], Some(2.0), &times)
            .expect("protocol simulation should succeed");

        let signal = trace
            .trace_of(&model.dynamics, "signal")
            .expect("signal is a named algebraic");

        assert_eq!(signal.len(), times.len());
        assert!((signal[2] - washout).abs() < 1e-6);
        assert!((signal[3] - washout * (-0.5f64).exp()).abs() < 1e-5);
        assert!((signal[4] - washout * (-4.0f64).exp()).abs() < 1e-3);

        // A washout between observation points must not shift the
        // stitched trace: the appended boundary column is dropped.
        let off_grid = 2.5;

        let trace = model
            .simulate_protocol(&[2.0, 0.5], &[1.0], Some(off_grid), &times)
            .expect("off-grid washout should succeed");

        let signal = trace
            .trace_of(&model.dynamics, "signal")
            .expect("signal is a named algebraic");

        assert_eq!(signal.len(), times.len());

        let at_step = 4.0 * (1.0 - (-1.25f64).exp());

        assert!((signal[3] - at_step * (-0.25f64).exp()).abs() < 1e-5);

        // t_step beyond the record reproduces the constant-input run.
        let constant = model
            .simulate_protocol(&[2.0, 0.5], &[1.0], Some(100.0), &times)
            .expect("late washout should succeed");

        let constant = constant
            .trace_of(&model.dynamics, "signal")
            .expect("signal is a named algebraic");

        assert!((constant[3] - 4.0 * (1.0 - (-1.5f64).exp())).abs() < 1e-5);
    }
}
