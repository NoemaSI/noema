//! The shipped maps are the specification: every known world — the
//! BLI assay, the tumour ladder, the detergent line, and the TCG-style
//! tablet line — must be expressible, validatable, and stable.

use std::path::{Path, PathBuf};

use smap::{SystemMap, map_hash, validate};

fn maps_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("maps")
}

fn load(file: &str) -> SystemMap {
    let path = maps_dir().join(file);

    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));

    SystemMap::from_json(&text).unwrap_or_else(|error| panic!("parsing {file}: {error}"))
}

#[test]
fn every_shipped_map_validates_and_hashes_stably() {
    let mut count = 0usize;

    for entry in std::fs::read_dir(maps_dir()).expect("maps directory") {
        let path = entry.expect("directory entry").path();

        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let text = std::fs::read_to_string(&path).expect("readable map");

        let map = SystemMap::from_json(&text)
            .unwrap_or_else(|error| panic!("parsing {}: {error}", path.display()));

        validate(&map).unwrap_or_else(|report| panic!("validating {}:\n{report}", path.display()));

        let reparsed = SystemMap::from_json(&map.to_json()).expect("round trip parses");

        assert_eq!(
            map_hash(&map),
            map_hash(&reparsed),
            "hash survives a JSON round trip: {}",
            path.display(),
        );

        count += 1;
    }

    assert!(count >= 4, "expected the four known worlds, found {count}");
}

#[test]
fn every_map_poses_discovery_questions() {
    for file in ["tablet_line.json", "toy_line.json", "tumor_t1.json", "binding.json"] {
        let map = load(file);

        let targets: Vec<&str> = map
            .discovery_targets()
            .iter()
            .map(|function| function.name.as_str())
            .collect();

        assert!(!targets.is_empty(), "{file} states no mechanism gaps: {targets:?}");
    }
}

#[test]
fn tablet_line_mirrors_the_csv_and_the_mermaid() {
    let map = load("tablet_line.json");

    validate(&map).expect("tablet line map");

    // The mermaid structure: two stages, five machines, a combiner,
    // an ambient coupling, a product sink.
    for name in [
        "Stage1",
        "Stage2",
        "Machine1",
        "Machine2",
        "Machine3",
        "Machine4",
        "Machine5",
        "Combiner",
    ] {
        assert!(map.element(name).is_some(), "{name} missing");
    }

    assert_eq!(map.element("Machine1").expect("m1").parent.as_deref(), Some("Stage1"));
    assert!(map.element("AmbientConditions").expect("ambient").external);

    // The mermaid arrows became flows: M1|M2|M3 -> combiner -> M4.
    let stage1 = map.flow("Stage1Product").expect("stage 1 flow");

    assert_eq!(stage1.edges.len(), 4);

    assert!(
        stage1
            .edges
            .iter()
            .any(|edge| edge.from == "Combiner.output" && edge.to == "Machine4.infeed")
    );

    // The CSV became signals: every dotted column binds somewhere.
    let raw = map.port("Machine1.raw").expect("raw port");

    assert_eq!(raw.signals.len(), 4);
    assert_eq!(
        raw.signals[0].channel.as_deref(),
        Some("Machine1.RawMaterial.Property1"),
    );

    assert_eq!(map.element("Machine1").expect("m1").signals.len(), 8);

    let out = map.port("Combiner.output").expect("combiner out");

    assert_eq!(out.signals.len(), 30, "15 measurements x actual+setpoint");

    // The discovery targets are the line's open mechanisms.
    let targets: Vec<&str> = map
        .discovery_targets()
        .iter()
        .map(|function| function.name.as_str())
        .collect();

    for gap in ["ShapeStage1", "Combine", "FinishStage2", "HoldTemperatures"] {
        assert!(targets.contains(&gap), "{gap} should be a discovery target");
    }
}

#[test]
fn toy_line_maps_the_buffers_as_elements_with_unknown_drain() {
    let map = load("toy_line.json");

    validate(&map).expect("toy line map");

    // The lag says inventory exists: the truth's two dynamic states
    // show up in the map as container elements with an unknown drain.
    assert_eq!(map.element("BagBuffer").expect("bags").kind, smap::ElementKind::Container);
    assert_eq!(map.element("BoxBuffer").expect("boxes").kind, smap::ElementKind::Container);

    let drain = map.function("BufferDrain").expect("drain function");

    assert_eq!(drain.status, smap::FunctionStatus::Unknown);
    assert_eq!(drain.allocated_to, vec!["BagBuffer", "BoxBuffer"]);

    // The throughput signature is traceable.
    let requirement = map.requirement("REQ_BOTTLENECK").expect("requirement");

    assert!(requirement.satisfies.contains(&"Pack".to_string()));
}

#[test]
fn tumor_map_carries_the_inferred_compartment() {
    let map = load("tumor_t1.json");

    validate(&map).expect("tumor map");

    let drug = map.element("DrugLevel").expect("drug");

    assert_eq!(
        drug.attributes.get("inferred").and_then(|v| v.as_bool()),
        Some(true),
        "the unobservable compartment must be marked as inferred",
    );

    // Its existence is earned through flows only: dose in, kill out.
    assert!(map.flow("Dose").expect("dose").edges.iter().any(|edge| edge.to == "DrugLevel.in"));
    assert!(map.flow("Killing").expect("kill").edges.iter().any(|edge| edge.from == "DrugLevel.effect"));

    // Behaviour: suppression then regrowth as named modes.
    assert!(map.state("Tumor.suppressed").is_some());
    assert!(map.state("Tumor.regrowing").is_some());
}

#[test]
fn binding_map_hands_off_to_the_skill_loop() {
    let map = load("binding.json");

    validate(&map).expect("binding map");

    let targets: Vec<&str> = map
        .discovery_targets()
        .iter()
        .map(|function| function.name.as_str())
        .collect();

    assert!(targets.contains(&"Associate"));
    assert!(targets.contains(&"Dissociate"));

    // The escape hatch works: a relation the core has no name for.
    assert!(!map.relations.is_empty());
}