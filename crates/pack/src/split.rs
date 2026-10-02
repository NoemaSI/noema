//! The visible/hidden conditioning split — fixed benchmark
//! infrastructure, part of "reality".
//!
//! Every subject appears on both sides: we hold out *observations*
//! from each subject (curves at held-out conditions), never whole
//! subjects. The LLM-facing structure is [`VisibleSubject`]; the
//! hidden side must never enter a discovery-loop context.
//!
//! Role assignment is harness code over the pack's [`SplitPolicy`]:
//! a pack declares which conditions are train/validation/hidden, and
//! this module enforces the separation.

use crate::data::Curve;
use crate::experiment::Condition;
use crate::pack::{SplitPolicy, Subject};

/// The data exposed to the model author for one subject: id, title,
/// public metadata, visible curves only.
#[derive(Debug, Clone)]
pub struct VisibleSubject {
    pub id: String,
    pub title: String,
    pub detail: Vec<(String, String)>,
    pub visible_curves: Vec<Curve>,
}

/// The data retained by the evaluator. The model author never receives
/// this structure.
#[derive(Debug, Clone)]
pub struct HeldOutSubject {
    pub id: String,
    pub title: String,
    pub detail: Vec<(String, String)>,
    pub held_out_curves: Vec<Curve>,
}

/// A benchmark split.
#[derive(Debug, Clone)]
pub struct BenchmarkDataset {
    pub discovery: Vec<VisibleSubject>,
    pub evaluation: Vec<HeldOutSubject>,
}

/// The three curve roles of one subject during evolution.
///
/// `train` feeds the fixed optimizer, `validation` is the selection
/// fitness (both inside the discovery-visible region), `hidden` is
/// scored only by the evaluator and never reaches an author-facing
/// structure.
#[derive(Debug, Clone, Default)]
pub struct CurveRoles {
    pub train: Vec<Curve>,
    pub validation: Vec<Curve>,
    pub hidden: Vec<Curve>,
}

/// Classify curves into roles by conditioning value.
pub fn split_roles(curves: &[Curve], policy: &SplitPolicy) -> CurveRoles {
    let role_of = |curve: &Curve| {
        let matches = |set: &[Condition]| {
            set.iter().any(|condition| curve.condition.matches(condition))
        };

        if matches(&policy.train) {
            Some(0)
        } else if matches(&policy.validation) {
            Some(1)
        } else if matches(&policy.hidden) {
            Some(2)
        } else {
            None
        }
    };

    let mut roles = CurveRoles::default();

    for curve in curves {
        match role_of(curve) {
            Some(0) => roles.train.push(curve.clone()),
            Some(1) => roles.validation.push(curve.clone()),
            Some(2) => roles.hidden.push(curve.clone()),
            _ => {}
        }
    }

    roles
}

/// Split every subject by conditioning value, per the pack's policy.
///
/// Subjects that do not contain at least one held-out curve cannot
/// participate in the split and are excluded.
pub fn split_subjects(
    subjects: Vec<Subject>,
    policy: &SplitPolicy,
) -> BenchmarkDataset {
    let mut discovery = Vec::new();
    let mut evaluation = Vec::new();

    for subject in subjects {
        let mut visible_curves = Vec::new();
        let mut held_out_curves = Vec::new();

        for curve in subject.curves {
            if policy
                .hidden
                .iter()
                .any(|condition| curve.condition.matches(condition))
            {
                held_out_curves.push(curve);
            } else {
                visible_curves.push(curve);
            }
        }

        if held_out_curves.is_empty() {
            continue;
        }

        discovery.push(VisibleSubject {
            id: subject.id.clone(),
            title: subject.title.clone(),
            detail: subject.detail.clone(),
            visible_curves,
        });

        evaluation.push(HeldOutSubject {
            id: subject.id,
            title: subject.title,
            detail: subject.detail,
            held_out_curves,
        });
    }

    BenchmarkDataset {
        discovery,
        evaluation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::data::RawCurve;

    fn curve(input: f64) -> Curve {
        Curve {
            condition: Condition::Level(input),
            raw: RawCurve {
                t: vec![0.0, 1.0],
                y: vec![0.0, 1.0],
            },
            session: 0,
            t_step: None,
            input_after: 0.0,
            drives: vec![],
            output_channel: None,
        }
    }

    fn subjects() -> Vec<Subject> {
        vec![
            Subject {
                id: "a".into(),
                title: "A".into(),
                detail: vec![("sequence".into(), "AAA".into())],
                curves: vec![curve(0.0), curve(31.6), curve(100.0), curve(316.2)],
            },
            Subject {
                id: "b".into(),
                title: "B".into(),
                detail: vec![("sequence".into(), "BBB".into())],
                // No held-out condition: excluded from the split.
                curves: vec![curve(0.0), curve(31.6)],
            },
        ]
    }

    fn policy() -> SplitPolicy {
        SplitPolicy::levels(&[0.0, 31.6], &[100.0], &[316.0, 1000.0])
    }

    #[test]
    fn every_subject_appears_on_both_sides() {
        let benchmark = split_subjects(subjects(), &policy());

        assert_eq!(benchmark.discovery.len(), 1);
        assert_eq!(benchmark.evaluation.len(), 1);

        assert_eq!(benchmark.discovery[0].visible_curves.len(), 3);
        assert_eq!(benchmark.evaluation[0].held_out_curves.len(), 1);

        assert_eq!(benchmark.discovery[0].id, "a");
        assert_eq!(benchmark.evaluation[0].id, "a");
    }

    #[test]
    fn held_out_split_survives_float_noise() {
        let benchmark = split_subjects(subjects(), &policy());

        assert_eq!(benchmark.evaluation[0].held_out_curves.len(), 1);
    }

    #[test]
    fn roles_partition_by_input_level() {
        let curves = vec![
            curve(0.0),
            curve(31.6),
            curve(31.6),
            curve(100.0),
            curve(316.2),
            curve(1000.0),
            curve(10.0), // in no role: unused
        ];

        let roles = split_roles(&curves, &policy());

        assert_eq!(roles.train.len(), 3);
        assert_eq!(roles.validation.len(), 1);
        assert_eq!(roles.hidden.len(), 2);
    }
}