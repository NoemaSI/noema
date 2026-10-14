//! The map validator: facts about a system map, all of them in one
//! pass so a repair round can fix them at once. Same spirit as
//! `mechanism_ir::validate` — the validator states facts, the human
//! (or the theorist) states verdicts.
//!
//! The rules encode the architecture discipline the representation is
//! built on:
//!
//! - one system root; everything internal hangs off it; external
//!   elements are boundary black boxes;
//! - flows terminate at *ports*, with matching directions and kinds;
//! - every declared port participates in at least one flow — an
//!   unwired interface is a claim the map does not use;
//! - every `unknown` function is allocated to at least one element
//!   *inside* the boundary: a discovery target must live somewhere we
//!   intend to explain.

use std::collections::{HashMap, HashSet};

use crate::model::{
    Direction, ElementKind, Flow, FunctionStatus, Relation, SystemMap, is_identifier,
};

/// Validate a system map against the IR contract.
pub fn validate(map: &SystemMap) -> Result<(), String> {
    let mut errors: Vec<String> = Vec::new();

    let require = |errors: &mut Vec<String>, ok: bool, message: String| {
        if !ok {
            errors.push(message);
        }
    };

    require(
        &mut errors,
        is_identifier(&map.name),
        format!("`name` {:?} is not an identifier", map.name),
    );

    validate_elements(map, &mut errors, &require);

    let port_index = validate_ports(map, &mut errors, &require);

    validate_flows(map, &port_index, &mut errors, &require);

    validate_functions(map, &mut errors, &require);

    validate_states(map, &mut errors, &require);

    validate_requirements(map, &mut errors, &require);

    validate_relations(map, &port_index, &mut errors, &require);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors
            .iter()
            .enumerate()
            .map(|(index, error)| format!("{}. {error}", index + 1))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

type Require<'a> = &'a dyn Fn(&mut Vec<String>, bool, String);

fn validate_elements(map: &SystemMap, errors: &mut Vec<String>, require: Require) {
    let mut names: HashSet<&str> = HashSet::new();

    for element in &map.elements {
        require(
            errors,
            is_identifier(&element.name),
            format!("element {element:?} is not an identifier"),
        );

        require(
            errors,
            names.insert(element.name.as_str()),
            format!("duplicate element name {:?}", element.name),
        );
    }

    // Boundary and hierarchy.
    let mut internal_roots = 0usize;

    for element in &map.elements {
        if let Some(parent) = &element.parent {
            require(
                errors,
                parent != &element.name,
                format!("element {:?} is its own parent", element.name),
            );

            match map.element(parent) {
                None => errors.push(format!(
                    "element {:?} has undeclared parent {parent:?}",
                    element.name,
                )),
                Some(parent_element) => require(
                    errors,
                    !parent_element.external,
                    format!(
                        "element {:?} hangs off external element {parent:?} — \
                         internal structure cannot live inside a black box",
                        element.name,
                    ),
                ),
            }
        } else if element.external {
            continue;
        } else {
            internal_roots += 1;

            require(
                errors,
                element.kind == ElementKind::System,
                format!(
                    "internal root element {:?} must be kind \"system\" \
                     (found {:?})",
                    element.name, element.kind,
                ),
            );
        }
    }

    require(
        errors,
        internal_roots == 1,
        format!(
            "exactly one internal root element (the system) is required \
             (found {internal_roots}); external elements may sit at the top \
             level as boundary black boxes",
        ),
    );

    // Ancestry cycles: every parent chain must reach a root.
    for element in &map.elements {
        let mut seen: HashSet<&str> = HashSet::new();

        seen.insert(element.name.as_str());

        let mut current = element.parent.as_deref();

        while let Some(name) = current {
            if !seen.insert(name) {
                errors.push(format!(
                    "element {:?} sits in a parent cycle",
                    element.name,
                ));

                break;
            }

            current = map
                .element(name)
                .and_then(|candidate| candidate.parent.as_deref());
        }
    }
}

fn validate_ports<'a>(
    map: &'a SystemMap,
    errors: &mut Vec<String>,
    require: Require,
) -> HashMap<String, &'a crate::model::Port> {
    let mut index: HashMap<String, &crate::model::Port> = HashMap::new();

    for element in &map.elements {
        let mut port_names: HashSet<&str> = HashSet::new();

        for port in &element.ports {
            require(
                errors,
                is_identifier(&port.name),
                format!(
                    "port {:?} on element {:?} is not an identifier",
                    port.name, element.name,
                ),
            );

            require(
                errors,
                port_names.insert(port.name.as_str()),
                format!(
                    "duplicate port {:?} on element {:?}",
                    port.name, element.name,
                ),
            );

            let reference = format!("{}.{}", element.name, port.name);

            require(
                errors,
                index.insert(reference.clone(), port).is_none(),
                format!("duplicate port reference {reference:?}"),
            );
        }

        check_signal_names(&element.name, &element.signals, errors, require);

        for port in &element.ports {
            check_signal_names(
                &format!("{}.{}", element.name, port.name),
                &port.signals,
                errors,
                require,
            );
        }
    }

    index
}

fn check_signal_names(
    owner: &str,
    signals: &[crate::model::Signal],
    errors: &mut Vec<String>,
    require: Require,
) {
    let mut names: HashSet<&str> = HashSet::new();

    for signal in signals {
        require(
            errors,
            is_identifier(&signal.name),
            format!("signal {:?} on {owner:?} is not an identifier", signal.name),
        );

        require(
            errors,
            names.insert(signal.name.as_str()),
            format!("duplicate signal {:?} on {owner:?}", signal.name),
        );
    }
}

fn validate_flows(
    map: &SystemMap,
    port_index: &HashMap<String, &crate::model::Port>,
    errors: &mut Vec<String>,
    require: Require,
) {
    let mut flow_names: HashSet<&str> = HashSet::new();

    let mut used_ports: HashSet<&str> = HashSet::new();

    for flow in &map.flows {
        require(
            errors,
            is_identifier(&flow.name),
            format!("flow {flow:?} is not an identifier"),
        );

        require(
            errors,
            flow_names.insert(flow.name.as_str()),
            format!("duplicate flow name {:?}", flow.name),
        );

        require(
            errors,
            !flow.edges.is_empty(),
            format!("flow {:?} has no edges", flow.name),
        );

        for edge in &flow.edges {
            require(
                errors,
                edge.from != edge.to,
                format!(
                    "flow {:?} contains the self-loop {:?}",
                    flow.name, edge.from,
                ),
            );

            let from = resolve_port(&edge.from, flow, port_index, errors);

            let to = resolve_port(&edge.to, flow, port_index, errors);

            if let Some(port) = from {
                require(
                    errors,
                    matches!(port.direction, Direction::Out | Direction::InOut),
                    format!(
                        "flow {:?} source {:?} is an {:?} port — flows leave \
                         out (or in_out) ports",
                        flow.name, edge.from, port.direction,
                    ),
                );

                require(
                    errors,
                    port.kind == flow.kind,
                    format!(
                        "flow {:?} ({:?}) leaves {:?} ({:?}) — kind mismatch",
                        flow.name, flow.kind, edge.from, port.kind,
                    ),
                );

                used_ports.insert(edge.from.as_str());
            }

            if let Some(port) = to {
                require(
                    errors,
                    matches!(port.direction, Direction::In | Direction::InOut),
                    format!(
                        "flow {:?} target {:?} is an {:?} port — flows enter \
                         in (or in_out) ports",
                        flow.name, edge.to, port.direction,
                    ),
                );

                require(
                    errors,
                    port.kind == flow.kind,
                    format!(
                        "flow {:?} ({:?}) enters {:?} ({:?}) — kind mismatch",
                        flow.name, flow.kind, edge.to, port.kind,
                    ),
                );

                used_ports.insert(edge.to.as_str());
            }
        }
    }

    // No dead interfaces.
    for reference in port_index.keys() {
        require(
            errors,
            used_ports.contains(reference.as_str()),
            format!(
                "port {reference:?} is touched by no flow — wire it into a \
                 flow or remove it",
            ),
        );
    }
}

fn resolve_port<'a>(
    reference: &str,
    flow: &Flow,
    port_index: &'a HashMap<String, &'a crate::model::Port>,
    errors: &mut Vec<String>,
) -> Option<&'a crate::model::Port> {
    match port_index.get(reference) {
        Some(port) => Some(*port),
        None => {
            errors.push(format!(
                "flow {:?} references unknown port {reference:?} \
                 (ports are \"element.port\")",
                flow.name,
            ));

            None
        }
    }
}

fn validate_functions(map: &SystemMap, errors: &mut Vec<String>, require: Require) {
    let mut names: HashSet<&str> = HashSet::new();

    let flow_names: HashSet<&str> = map
        .flows
        .iter()
        .map(|flow| flow.name.as_str())
        .collect();

    for function in &map.functions {
        require(
            errors,
            is_identifier(&function.name),
            format!("function {function:?} is not an identifier"),
        );

        require(
            errors,
            names.insert(function.name.as_str()),
            format!("duplicate function name {:?}", function.name),
        );

        if let Some(parent) = &function.parent {
            require(
                errors,
                parent != &function.name && map.function(parent).is_some(),
                format!(
                    "function {:?} has undeclared parent {parent:?}",
                    function.name,
                ),
            );
        }

        for target in function.reads.iter().chain(&function.writes) {
            require(
                errors,
                flow_names.contains(target.as_str()),
                format!(
                    "function {:?} references undeclared flow {target:?} \
                     (reads/writes name flows)",
                    function.name,
                ),
            );
        }

        for element_name in &function.allocated_to {
            match map.element(element_name) {
                None => errors.push(format!(
                    "function {:?} is allocated to undeclared element {element_name:?}",
                    function.name,
                )),
                Some(element) => require(
                    errors,
                    function.status != FunctionStatus::Unknown || !element.external,
                    format!(
                        "function {:?} is an open mechanism question but is \
                         allocated to external element {element_name:?} — \
                         discovery targets must live inside the boundary",
                        function.name,
                    ),
                ),
            }
        }

        require(
            errors,
            function.status != FunctionStatus::Unknown || !function.allocated_to.is_empty(),
            format!(
                "unknown function {:?} must be allocated to at least one \
                 element: a mechanism question must be posed somewhere",
                function.name,
            ),
        );
    }

    // Function ancestry cycles.
    for function in &map.functions {
        let mut seen: HashSet<&str> = HashSet::new();

        seen.insert(function.name.as_str());

        let mut current = function.parent.as_deref();

        while let Some(name) = current {
            if !seen.insert(name) {
                errors.push(format!(
                    "function {:?} sits in a parent cycle",
                    function.name,
                ));

                break;
            }

            current = map
                .function(name)
                .and_then(|candidate| candidate.parent.as_deref());
        }
    }
}

fn validate_states(map: &SystemMap, errors: &mut Vec<String>, require: Require) {
    let mut references: HashSet<String> = HashSet::new();

    for state in &map.states {
        require(
            errors,
            is_identifier(&state.name),
            format!("state {:?} is not an identifier", state.name),
        );

        require(
            errors,
            map.element(&state.owner).is_some(),
            format!(
                "state {:?} is owned by undeclared element {:?}",
                state.name, state.owner,
            ),
        );

        require(
            errors,
            references.insert(state.reference()),
            format!("duplicate state {:?}", state.reference()),
        );
    }

    for transition in &map.transitions {
        for reference in [&transition.from, &transition.to] {
            require(
                errors,
                references.contains(reference.as_str()),
                format!(
                    "transition references undeclared state {reference:?} \
                     (states are referenced \"owner.name\")",
                ),
            );
        }

        require(
            errors,
            transition.from != transition.to,
            format!(
                "transition from {:?} to itself",
                transition.from,
            ),
        );
    }
}

fn validate_requirements(map: &SystemMap, errors: &mut Vec<String>, require: Require) {
    let mut ids: HashSet<&str> = HashSet::new();

    for requirement in &map.requirements {
        require(
            errors,
            is_identifier(&requirement.id),
            format!("requirement id {:?} is not an identifier", requirement.id),
        );

        require(
            errors,
            ids.insert(requirement.id.as_str()),
            format!("duplicate requirement id {:?}", requirement.id),
        );

        require(
            errors,
            !requirement.text.trim().is_empty(),
            format!("requirement {:?} has no text", requirement.id),
        );

        for target in &requirement.satisfies {
            require(
                errors,
                map.element(target).is_some() || map.function(target).is_some(),
                format!(
                    "requirement {:?} satisfied_by undeclared element or \
                     function {target:?}",
                    requirement.id,
                ),
            );
        }
    }
}

fn validate_relations(
    map: &SystemMap,
    port_index: &HashMap<String, &crate::model::Port>,
    errors: &mut Vec<String>,
    require: Require,
) {
    let mut namespace: HashSet<&str> = HashSet::new();

    namespace.insert(map.name.as_str());

    for element in &map.elements {
        namespace.insert(element.name.as_str());
    }

    for function in &map.functions {
        namespace.insert(function.name.as_str());
    }

    for flow in &map.flows {
        namespace.insert(flow.name.as_str());
    }

    for state in &map.states {
        namespace.insert(state.name.as_str());
    }

    for requirement in &map.requirements {
        namespace.insert(requirement.id.as_str());
    }

    for reference in port_index.keys() {
        namespace.insert(reference.as_str());
    }

    for relation in &map.relations {
        let (from, to) = relation.endpoints();

        for endpoint in [from, to] {
            require(
                errors,
                namespace.contains(endpoint),
                format!(
                    "{} relation references unknown endpoint {endpoint:?}",
                    relation_name(relation),
                ),
            );
        }

        if let Relation::Custom { relation, .. } = relation {
            require(
                errors,
                !relation.trim().is_empty(),
                "custom relation needs a non-empty `relation` name".to_string(),
            );
        }
    }
}

fn relation_name(relation: &Relation) -> &'static str {
    match relation {
        Relation::DependsOn { .. } => "depends_on",
        Relation::Precedes { .. } => "precedes",
        Relation::TracesTo { .. } => "traces_to",
        Relation::Custom { .. } => "custom",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Function;

    const VALID: &str = r#"{
        "name": "M",
        "elements": [
            {"name": "S", "kind": "system"},
            {"name": "A", "kind": "hardware", "parent": "S",
             "ports": [{"name": "out", "direction": "out", "kind": "data"}]},
            {"name": "B", "kind": "hardware", "parent": "S",
             "ports": [{"name": "in", "direction": "in", "kind": "data"}]}
        ],
        "flows": [{"name": "F", "kind": "data",
                   "edges": [{"from": "A.out", "to": "B.in"}]}]
    }"#;

    fn minimal() -> SystemMap {
        SystemMap::from_json(VALID).expect("the minimal map parses")
    }

    #[test]
    fn minimal_map_validates() {
        validate(&minimal()).expect("minimal map is valid");
    }

    #[test]
    fn unknown_port_references_are_rejected() {
        let mut map = minimal();

        map.flows[0].edges[0].to = "B.ghost".to_string();

        let message = validate(&map).unwrap_err();

        assert!(message.contains("unknown port"), "{message}");
    }

    #[test]
    fn direction_is_enforced() {
        let mut map = minimal();

        // Flow entering an out port.
        map.flows[0].edges[0].to = "A.out".to_string();

        let message = validate(&map).unwrap_err();

        assert!(message.contains("enter"), "{message}");
    }

    #[test]
    fn interface_kind_mismatch_is_rejected() {
        let mut map = minimal();

        map.flows[0].kind = crate::model::InterfaceKind::Energy;

        let message = validate(&map).unwrap_err();

        assert!(message.contains("kind mismatch"), "{message}");
    }

    #[test]
    fn dead_ports_are_rejected() {
        let mut map = minimal();

        let b = map.elements.iter_mut().find(|e| e.name == "B").unwrap();

        b.ports.push(crate::model::Port {
            name: "unused".to_string(),
            direction: Direction::In,
            kind: crate::model::InterfaceKind::Data,
            signals: vec![],
            description: String::new(),
            attributes: Default::default(),
        });

        let message = validate(&map).unwrap_err();

        assert!(message.contains("touched by no flow"), "{message}");
    }

    #[test]
    fn exactly_one_internal_root() {
        let mut map = minimal();

        map.elements.push(crate::model::Element {
            name: "Orphan".to_string(),
            kind: ElementKind::Hardware,
            parent: None,
            external: false,
            ports: vec![],
            signals: vec![],
            description: String::new(),
            attributes: Default::default(),
        });

        let message = validate(&map).unwrap_err();

        assert!(message.contains("internal root"), "{message}");

        // External top-level black boxes are fine.
        let mut map = minimal();

        map.elements.push(crate::model::Element {
            name: "World".to_string(),
            kind: ElementKind::Resource,
            parent: None,
            external: true,
            ports: vec![],
            signals: vec![],
            description: String::new(),
            attributes: Default::default(),
        });

        validate(&map).expect("external roots are allowed");
    }

    #[test]
    fn parent_cycles_are_rejected() {
        let mut map = minimal();

        for element in &mut map.elements {
            if element.name == "A" {
                element.parent = Some("B".to_string());
            } else if element.name == "B" {
                element.parent = Some("A".to_string());
            }
        }

        let message = validate(&map).unwrap_err();

        assert!(message.contains("cycle"), "{message}");
    }

    #[test]
    fn discovery_targets_live_inside_the_boundary() {
        let mut map = minimal();

        map.elements.push(crate::model::Element {
            name: "World".to_string(),
            kind: ElementKind::Software,
            parent: None,
            external: true,
            ports: vec![],
            signals: vec![],
            description: String::new(),
            attributes: Default::default(),
        });

        // Unknown, unallocated.
        map.functions.push(Function {
            name: "Mystery".to_string(),
            ..Default::default()
        });

        let message = validate(&map).unwrap_err();

        assert!(message.contains("posed somewhere"), "{message}");

        // Unknown, allocated to a black box.
        map.functions[0].allocated_to = vec!["World".to_string()];

        let message = validate(&map).unwrap_err();

        assert!(message.contains("inside the boundary"), "{message}");

        // Unknown, allocated inside: the map is now asking a question.
        map.functions[0].allocated_to = vec!["A".to_string()];

        validate(&map).expect("a grounded discovery target is fine");
    }

    #[test]
    fn flow_references_and_relations_resolve() {
        let mut map = minimal();

        map.functions.push(Function {
            name: "Send".to_string(),
            allocated_to: vec!["A".to_string()],
            writes: vec!["Ghost".to_string()],
            status: FunctionStatus::Observed,
            ..Default::default()
        });

        let message = validate(&map).unwrap_err();

        assert!(message.contains("undeclared flow"), "{message}");

        let mut map = minimal();

        map.relations.push(Relation::Custom {
            relation: "realises".to_string(),
            from: "A".to_string(),
            to: "ghost.port".to_string(),
        });

        let message = validate(&map).unwrap_err();

        assert!(message.contains("unknown endpoint"), "{message}");

        // Port references are in the namespace.
        map.relations[0] = Relation::Custom {
            relation: "realises".to_string(),
            from: "A".to_string(),
            to: "B.in".to_string(),
        };

        validate(&map).expect("relations may reference ports");
    }

    #[test]
    fn states_are_owned_and_transitions_resolve() {
        let mut map = minimal();

        map.states.push(crate::model::State {
            name: "running".to_string(),
            owner: "Ghost".to_string(),
            description: String::new(),
            attributes: Default::default(),
        });

        let message = validate(&map).unwrap_err();

        assert!(message.contains("undeclared element"), "{message}");

        let mut map = minimal();

        map.states.push(crate::model::State {
            name: "running".to_string(),
            owner: "A".to_string(),
            description: String::new(),
            attributes: Default::default(),
        });

        map.transitions.push(crate::model::Transition {
            from: "A.running".to_string(),
            to: "A.idle".to_string(),
            trigger: None,
            guard: None,
        });

        let message = validate(&map).unwrap_err();

        assert!(message.contains("undeclared state"), "{message}");
    }
}