//! Deterministic mermaid rendering of a system map.
//!
//! The fixed house style:
//!
//! - `flowchart LR`, bare node ids only — **no shapes**;
//! - every element that contains others becomes a `subgraph`, so the
//!   decomposition boxes (stages, subsystems) are the picture;
//! - leaves are declared as bare identifiers inside their parent box;
//! - external elements are top-level nodes outside the system box;
//! - every flow edge becomes `from -->|flow| to` between *elements*
//!   (ports are hidden, parallel port pairs deduped);
//! - `environment` flows are dashed, everything else solid;
//! - the map's discovery targets are carried as `%%` comments, so a
//!   rendered diagram still states what is unknown.

use std::collections::BTreeSet;
use std::fmt;

use crate::model::{Element, InterfaceKind, SystemMap};

/// Diagram orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// Left to right (the house default).
    #[default]
    LR,

    /// Top down.
    TD,
}

impl Layout {
    /// Parse `lr`/`td`, either case.
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "lr" => Some(Layout::LR),
            "td" => Some(Layout::TD),
            _ => None,
        }
    }
}

impl fmt::Display for Layout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Layout::LR => "LR",
            Layout::TD => "TD",
        })
    }
}

/// Render a system map to mermaid text in the default layout.
pub fn to_mermaid(map: &SystemMap) -> String {
    to_mermaid_with(map, Layout::default())
}

/// Render a system map to mermaid text in the given layout.
pub fn to_mermaid_with(map: &SystemMap, layout: Layout) -> String {
    let mut out = format!("flowchart {layout}\n");

    // Boundary black boxes sit outside the system box.
    for element in map.elements.iter().filter(|element| element.external) {
        out.push_str(&format!("  {}\n", element.name));
    }

    if let Some(system) = map.system() {
        render_element(map, system, 1, &mut out);
    }

    render_flows(map, &mut out);

    render_discovery_targets(map, &mut out);

    out
}

fn render_element(map: &SystemMap, element: &Element, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);

    let children: Vec<&Element> = map
        .elements
        .iter()
        .filter(|candidate| candidate.parent.as_deref() == Some(element.name.as_str()))
        .collect();

    if children.is_empty() {
        out.push_str(&format!("{pad}{}\n", element.name));

        return;
    }

    out.push_str(&format!("{pad}subgraph {}\n", element.name));

    for child in children {
        render_element(map, child, depth + 1, out);
    }

    out.push_str(&format!("{pad}end\n"));
}

fn render_flows(map: &SystemMap, out: &mut String) {
    let mut seen: BTreeSet<(&str, &str, &str)> = BTreeSet::new();

    for flow in &map.flows {
        let arrow = if flow.kind == InterfaceKind::Environment {
            "-.->"
        } else {
            "-->"
        };

        for edge in &flow.edges {
            let Some((from, _)) = edge.from.split_once('.') else {
                continue;
            };

            let Some((to, _)) = edge.to.split_once('.') else {
                continue;
            };

            if from == to || !seen.insert((flow.name.as_str(), from, to)) {
                continue;
            }

            out.push_str(&format!("  {from} {arrow}|{}| {to}\n", flow.name));
        }
    }
}

fn render_discovery_targets(map: &SystemMap, out: &mut String) {
    let targets = map.discovery_targets();

    if targets.is_empty() {
        return;
    }

    out.push_str("  %% discovery targets:\n");

    for function in targets {
        out.push_str(&format!(
            "  %%   {} (allocated: {})\n",
            function.name,
            function.allocated_to.join(", "),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tablet_line() -> SystemMap {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("maps/tablet_line.json");

        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));

        SystemMap::from_json(&text).expect("tablet line map parses")
    }

    #[test]
    fn renders_the_boxed_structure_without_shapes() {
        let mermaid = to_mermaid(&tablet_line());

        assert!(mermaid.starts_with("flowchart LR\n"));

        // The decomposition boxes: stages nest inside the system box.
        assert!(mermaid.contains("subgraph TabletLine"), "{mermaid}");
        assert!(mermaid.contains("subgraph Stage1"), "{mermaid}");
        assert!(mermaid.contains("subgraph Stage2"), "{mermaid}");

        // The mermaid arrows, element-to-element, flow-labelled.
        assert!(mermaid.contains("Machine1 -->|Stage1Product| Combiner"), "{mermaid}");
        assert!(mermaid.contains("Combiner -->|Stage1Product| Machine4"), "{mermaid}");
        assert!(mermaid.contains("Machine5 -->|Stage2Product| Product"), "{mermaid}");

        // Ambient coupling stays dashed, into the stage box.
        assert!(
            mermaid.contains("AmbientConditions -.->|AmbientInfluence| Stage1"),
            "{mermaid}",
        );

        // The questions survive the rendering.
        assert!(mermaid.contains("%%   ShapeStage1"), "{mermaid}");

        // "No shapes": not a single node decorator on any diagram line
        // (only the %% comment lines may carry parentheses).
        for line in mermaid.lines().filter(|line| !line.trim_start().starts_with("%%")) {
            assert!(!line.contains('[') && !line.contains('('), "shape on {line:?}");
        }
    }

    #[test]
    fn layout_defaults_to_lr_and_td_is_selectable() {
        let map = tablet_line();

        assert!(to_mermaid(&map).starts_with("flowchart LR\n"));
        assert!(to_mermaid_with(&map, Layout::TD).starts_with("flowchart TD\n"));

        assert_eq!(Layout::parse("td"), Some(Layout::TD));
        assert_eq!(Layout::parse("LR"), Some(Layout::LR));
        assert_eq!(Layout::parse("bt"), None);
    }

    #[test]
    fn dedupes_parallel_port_edges() {
        let map = tablet_line();

        let mermaid = to_mermaid(&map);

        let count = mermaid
            .lines()
            .filter(|line| line.contains("RawMaterial -->|RawMaterialFeed|"))
            .count();

        assert_eq!(count, 3, "one arrow per machine, fan-out preserved:\n{mermaid}");
    }
}