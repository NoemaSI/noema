//! The harness-side curve record.
//!
//! A pack ingests its own data format and hands the loop flat
//! [`Curve`] records: one measured response series at one
//! conditioning value, with its protocol (step time and post-step
//! level) or its full recorded drives attached. Everything
//! downstream — splitting, fitting, scoring, plotting — only ever
//! sees this shape.
//!
//! Two derived quantities are computed once at load time and are data,
//! never model degrees of freedom:
//!
//! - [`quantize`]: input levels recorded by a dilution series carry
//!   representation noise (`316.0` vs `316.2`); bucketing keeps the
//!   split honest.
//! - [`detect_step_time`]: the protocol switch time (for a binding
//!   assay, the tip move from analyte to buffer) inferred from the
//!   shape of the session's own traces.

use serde::{Deserialize, Serialize};

/// One measured response series at one conditioning value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    /// The conditioning value this curve was measured at: the
    /// numeric drive level (single-channel problems) or the regime
    /// tag the split roles on.
    pub condition: crate::experiment::Condition,

    /// The raw time series (of the curve's output channel).
    pub raw: RawCurve,

    /// Index of the source measurement (session) this curve came
    /// from, in the order the pack references them: one session is
    /// one instrument run holding a full input series.
    #[serde(default)]
    pub session: usize,

    /// Protocol step time (s): the point where the recorded trace
    /// stops rising and enters the second phase, detected once per
    /// session and shared by every curve of that session. `None` when
    /// no step is detectable (monotone or flat records).
    #[serde(default)]
    pub t_step: Option<f64>,

    /// Drive level after `t_step` — the pack's protocol semantics
    /// (a binding pack's washout into buffer means `0.0`). The
    /// harness replays it; it is never fitted. Meaningless when
    /// `t_step` is `None`.
    #[serde(default)]
    pub input_after: f64,

    /// Recorded drives for multi-channel problems: when non-empty,
    /// these are the experiment's drives verbatim (the primary
    /// channel's `condition`/`t_step`/`input_after` staircase is not
    /// synthesized). Single-channel packs leave this empty.
    #[serde(default)]
    pub drives: Vec<crate::experiment::InputDrive>,

    /// The output channel this curve's `raw` series belongs to.
    /// Defaults to the lexicon's primary output.
    #[serde(default)]
    pub output_channel: Option<String>,
}

/// Raw response time series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCurve {
    pub t: Vec<f64>,
    pub y: Vec<f64>,
}

/// Input levels are produced by a serial dilution (or any geometric
/// series); bucket them logarithmically so counts and role
/// assignments do not fragment over representation noise.
pub fn quantize(value: f64) -> f64 {
    if value == 0.0 {
        return 0.0;
    }

    let exponent = value.abs().log10().round();

    let scale = 10f64.powf(exponent);

    (value / scale * 100.0).round() / 100.0 * scale
}

/// Detect the protocol step time of one session (one instrument run
/// holding a full input series).
///
/// The program holds the probe at the drive level, then switches
/// (for a binding assay: moves the tip into buffer); every trace of
/// the session switches at the same moment. The data format does not
/// record that moment, so it is estimated from the session's
/// strongest trace: smooth the response, take the last local maximum,
/// and accept it only when the signal then declines by a clear
/// margin. Monotone-rise or flat sessions (and pure decay windows
/// that start at their maximum) yield `None`, which keeps the
/// constant-input simulation.
///
/// NOTE: this value is data, not a model degree of freedom: it is
/// computed once at load time from the raw trace, shared by every
/// curve of the session, and never seen by the optimizer.
pub fn detect_step_time(curves: &[Curve]) -> Option<f64> {
    // The highest-amplitude trace gives the most reliable peak.
    let best = curves
        .iter()
        .filter(|curve| curve.condition.level().unwrap_or(1.0) > 0.0)
        .filter(|curve| curve.raw.t.len() >= 20)
        .max_by(|a, b| amplitude(&a.raw.y).total_cmp(&amplitude(&b.raw.y)))?;

    let t = &best.raw.t;
    let smoothed = smooth(&best.raw.y, 11);

    let peak_index = smoothed
        .iter()
        .enumerate()
        .max_by(|(_, x), (_, y)| x.total_cmp(y))?
        .0;

    // A step at the very start or very end is not usable: the
    // simulation cannot resolve a step with no phase on one side.
    if peak_index == 0 || peak_index + 1 >= t.len() {
        return None;
    }

    let peak = smoothed[peak_index];

    if peak < 0.1 {
        return None;
    }

    // Mean of the last tenth of the record: the post-step level.
    let tail_start = t.len() - (t.len() / 10).max(1);
    let tail: f64 = smoothed[tail_start..].iter().sum::<f64>()
        / (smoothed.len() - tail_start) as f64;

    // Require a real, sustained decline before trusting the peak as
    // a protocol step; noise around a plateau must not qualify.
    if peak - tail < 0.15 * peak {
        return None;
    }

    Some(t[peak_index])
}

fn amplitude(y: &[f64]) -> f64 {
    let max = y.iter().fold(f64::MIN, |a, &b| a.max(b));
    let min = y.iter().fold(f64::MAX, |a, &b| a.min(b));
    max - min
}

/// Centered moving average, edge-clamped.
fn smooth(y: &[f64], window: usize) -> Vec<f64> {
    let half = window / 2;

    (0..y.len())
        .map(|i| {
            let start = i.saturating_sub(half);
            let end = (i + half + 1).min(y.len());
            y[start..end].iter().sum::<f64>() / (end - start) as f64
        })
        .collect()
}

/// Whether missing data files may be downloaded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fetch {
    /// Only read the local cache; error on gaps.
    Offline,

    /// Download missing files into the cache.
    Online,
}

/// Read one remote file from the cache, downloading it if allowed and
/// absent. Cache entries are named by the URL's file name (or an FNV
/// hash of the URL when it has none), so the benchmark runs offline
/// after the first fetch.
pub fn cached_body(
    url: &str,
    cache_dir: &std::path::Path,
    fetch: Fetch,
) -> anyhow::Result<String> {
    use anyhow::{Context, bail};

    let name = url
        .rsplit('/')
        .next()
        .filter(|name| name.ends_with(".json"))
        .map(str::to_string)
        .unwrap_or_else(|| format!("{:016x}.json", fnv1a(url.as_bytes())));

    let path = cache_dir.join(&name);

    if let Ok(body) = std::fs::read_to_string(&path) {
        return Ok(body);
    }

    if fetch == Fetch::Offline {
        bail!(
            "data file {name} is not in the cache ({}) and offline mode \
             does not fetch",
            cache_dir.display(),
        );
    }

    let body = download(url)?;

    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("creating cache dir {}", cache_dir.display()))?;

    std::fs::write(&path, &body)
        .with_context(|| format!("writing cache entry {}", path.display()))?;

    Ok(body)
}

/// Ensure one remote file is in the cache, downloading it if absent.
pub fn ensure_cached(url: &str, cache_dir: &std::path::Path) -> anyhow::Result<()> {
    cached_body(url, cache_dir, Fetch::Online).map(|_| ())
}

/// Blocking fetch of one remote file.
fn download(url: &str) -> anyhow::Result<String> {
    use anyhow::{anyhow, bail};

    let mut response = ureq::get(url)
        .call()
        .map_err(|error| anyhow!("requesting {url}: {error}"))?;

    if !response.status().is_success() {
        bail!("data request returned HTTP {} for {url}", response.status());
    }

    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| anyhow!("reading body from {url}: {error}"))?;

    Ok(body)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;

    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantization_groups_dilution_values() {
        assert_eq!(quantize(0.0), 0.0);
        assert_eq!(quantize(316.0), quantize(316.2));
        assert_eq!(quantize(1000.0), quantize(1000.0001));
        assert_ne!(quantize(316.0), quantize(100.0));
    }

    fn synthetic_curve(input: f64, t: Vec<f64>, y: Vec<f64>) -> Curve {
        Curve {
            condition: crate::experiment::Condition::Level(input),
            raw: RawCurve { t, y },
            session: 0,
            t_step: None,
            input_after: 0.0,
            drives: vec![],
            output_channel: None,
        }
    }

    fn two_phase_session() -> Vec<Curve> {
        let t: Vec<f64> = (0..=60).map(|second| second as f64).collect();

        let y: Vec<f64> = t
            .iter()
            .map(|&time| {
                if time <= 30.0 {
                    time / 3.0
                } else {
                    10.0 - (time - 30.0) * 8.0 / 30.0
                }
            })
            .collect();

        vec![synthetic_curve(100.0, t, y)]
    }

    #[test]
    fn detects_step_at_the_peak_of_a_two_phase_trace() {
        let t_step = detect_step_time(&two_phase_session())
            .expect("rise-then-decay is a step");

        assert!((t_step - 30.0).abs() <= 5.0, "detected {t_step}, expected ~30");
    }

    #[test]
    fn monotone_rise_and_flat_records_have_no_step() {
        let t: Vec<f64> = (0..=60).map(|second| second as f64).collect();

        let rising = synthetic_curve(100.0, t.clone(), t.clone());
        assert_eq!(detect_step_time(&[rising]), None);

        let flat = synthetic_curve(100.0, t.clone(), vec![5.0; t.len()]);
        assert_eq!(detect_step_time(&[flat]), None);
    }

    #[test]
    fn pure_decay_window_starting_at_max_has_no_step() {
        let t: Vec<f64> = (0..=60).map(|second| second as f64).collect();

        let decay = synthetic_curve(
            100.0,
            t.clone(),
            t.iter().map(|&time| 10.0 * (-time / 20.0).exp()).collect(),
        );

        assert_eq!(detect_step_time(&[decay]), None);
    }

    #[test]
    fn short_and_zero_input_curves_are_not_steps() {
        let t: Vec<f64> = (0..10).map(|second| second as f64).collect();

        let short = synthetic_curve(100.0, t.clone(), vec![1.0; t.len()]);
        assert_eq!(detect_step_time(&[short]), None);

        let mut zero = two_phase_session();
        zero[0].condition = crate::experiment::Condition::Level(0.0);
        assert_eq!(detect_step_time(&zero), None);
    }
}