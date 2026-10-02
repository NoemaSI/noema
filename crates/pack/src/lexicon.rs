//! The pack's vocabulary: the channels the static machinery must know
//! but never invent.
//!
//! The IR, the lowerer, and the prompt templates are domain-generic,
//! but they still have to name the measured drive and response
//! quantities, and they must tell the LLM what the numbers mean. A
//! pack declares exactly that here: every input channel the
//! experiments drive (any number, varying arbitrarily over time —
//! the harness replays recorded traces), and every output channel the
//! mechanisms predict. Everything else about the problem lives in the
//! pack.

use serde::{Deserialize, Serialize};

/// One declared signal channel: an input the experiments drive, or an
/// output the mechanisms predict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    /// Modelica identifier — becomes `input Real <name>` /
    /// `output Real <name>` in every lowered model. Must not collide
    /// with any name the mechanism declares.
    pub name: String,

    /// Human-readable units, e.g. `"nM"`. Used in plots and prompts
    /// only; the harness never converts numbers.
    pub units: String,

    /// One-paragraph description of what the quantity is, injected
    /// into the theorist prompt.
    pub description: String,
}

/// The vocabulary of one problem domain: its input and output
/// channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lexicon {
    /// Every channel the experiments drive. The first is the
    /// *primary* channel — the one [`crate::data::Curve`]s condition
    /// and drive by default (single-channel problems declare
    /// exactly one).
    pub inputs: Vec<Channel>,

    /// Every channel the mechanisms must predict. The loss and the
    /// RMSEs are computed over exactly these.
    pub outputs: Vec<Channel>,

    /// Optional descriptive roles an entity may carry (e.g.
    /// `"immobilized"` for the surface-bound species of a BLI assay).
    /// Purely descriptive: the lowerer treats every non-driven entity
    /// identically; roles exist so prompts and reports can speak the
    /// domain's language.
    pub roles: Vec<String>,

    /// Free-form domain notes appended to the theorist prompt (unit
    /// conventions, measurement quirks, anything the model author
    /// must know).
    pub notes: String,
}

impl Lexicon {
    /// The primary input channel (validated to exist).
    pub fn primary_input(&self) -> &Channel {
        &self.inputs[0]
    }

    /// The primary output channel (validated to exist).
    pub fn primary_output(&self) -> &Channel {
        &self.outputs[0]
    }

    pub fn input(&self, name: &str) -> Option<&Channel> {
        self.inputs.iter().find(|channel| channel.name == name)
    }

    pub fn output(&self, name: &str) -> Option<&Channel> {
        self.outputs.iter().find(|channel| channel.name == name)
    }

    /// Validate that the vocabulary can drive the static machinery.
    pub fn validate(&self) -> Result<(), String> {
        if self.inputs.is_empty() {
            return Err("declare at least one input channel".to_string());
        }

        if self.outputs.is_empty() {
            return Err("declare at least one output channel".to_string());
        }

        let mut names = Vec::new();

        for channel in self.inputs.iter().chain(&self.outputs) {
            if !is_modelica_identifier(&channel.name) {
                return Err(format!(
                    "channel name {:?} is not a valid Modelica identifier",
                    channel.name,
                ));
            }

            for reserved in [
                "signal", "time", "der", "Real", "input", "output", "model", "end",
                "equation", "parameter",
            ] {
                // "signal" is a conventional output name, so it is
                // only rejected for inputs.
                if channel.name == reserved
                    && !(self.outputs.contains(channel) && reserved == "signal")
                {
                    return Err(format!(
                        "channel name {:?} collides with the reserved \
                         Modelica word {reserved:?}",
                        channel.name,
                    ));
                }
            }

            if names.contains(&channel.name) {
                return Err(format!("duplicate channel name {:?}", channel.name));
            }

            names.push(channel.name.clone());
        }

        for role in &self.roles {
            if role.trim().is_empty() {
                return Err("roles must not be blank".to_string());
            }
        }

        Ok(())
    }
}

/// True for `[A-Za-z_][A-Za-z0-9_]*` (the subset of Modelica identifiers
/// the machinery needs).
pub fn is_modelica_identifier(name: &str) -> bool {
    let mut chars = name.chars();

    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }

    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(name: &str, units: &str) -> Channel {
        Channel {
            name: name.to_string(),
            units: units.to_string(),
            description: "test channel".to_string(),
        }
    }

    fn lexicon() -> Lexicon {
        Lexicon {
            inputs: vec![channel("concentration", "nM")],
            outputs: vec![channel("signal", "nm")],
            roles: vec!["immobilized".to_string()],
            notes: String::new(),
        }
    }

    #[test]
    fn vocabulary_validates() {
        assert_eq!(lexicon().validate(), Ok(()));

        // Multi-channel vocabulations validate too.
        let mut multi = lexicon();
        multi.inputs.push(channel("valve", "%"));
        multi.outputs.push(channel("temperature", "K"));

        assert_eq!(multi.validate(), Ok(()));
    }

#[test]
fn bad_channel_names_are_rejected() {
    let mut bad = lexicon();
    bad.inputs[0].name = "3H".to_string();
    assert!(bad.validate().is_err());

    let mut bad = lexicon();
    bad.inputs[0].name = "signal".to_string();
    assert!(bad.validate().is_err());

    // Empty tables are rejected.
    let mut bad = lexicon();
    bad.inputs.clear();
    assert!(bad.validate().is_err());

    // Duplicate names across the two tables are rejected.
    let mut bad = lexicon();
    bad.inputs.push(channel("signal", "nm"));
    assert!(bad.validate().is_err());
}

    #[test]
    fn identifier_rule() {
        assert!(is_modelica_identifier("concentration"));
        assert!(is_modelica_identifier("_a1"));
        assert!(!is_modelica_identifier(""));
        assert!(!is_modelica_identifier("1a"));
        assert!(!is_modelica_identifier("a-b"));
    }
}