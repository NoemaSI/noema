//! `smap schema`             — print the JSON schema an agent authors against.
//! `smap validate <file>…`   — parse and validate map files, report facts.
//! `smap mermaid <file>`     — render one map to mermaid (--layout=lr|td, default lr).

use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().collect();

    match arguments.get(1).map(String::as_str) {
        Some("schema") => {
            println!("{}", smap::schema_json());

            ExitCode::SUCCESS
        }
        Some("mermaid") => {
            let mut layout = smap::Layout::default();

            let mut file: Option<String> = None;

            for argument in &arguments[2..] {
                if let Some(requested) = argument.strip_prefix("--layout=") {
                    match smap::Layout::parse(requested) {
                        Some(parsed) => layout = parsed,
                        None => {
                            eprintln!("unknown layout {requested:?} (expected lr or td)");

                            return ExitCode::FAILURE;
                        }
                    }
                } else if file.is_none() {
                    file = Some(argument.clone());
                } else {
                    eprintln!("usage: smap mermaid [--layout=lr|td] <map.json>");

                    return ExitCode::FAILURE;
                }
            }

            let Some(file) = file else {
                eprintln!("usage: smap mermaid [--layout=lr|td] <map.json>");

                return ExitCode::FAILURE;
            };

            let text = match fs::read_to_string(&file) {
                Ok(text) => text,
                Err(error) => {
                    eprintln!("{file}: cannot read: {error}");

                    return ExitCode::FAILURE;
                }
            };

            let map = match smap::SystemMap::from_json(&text) {
                Ok(map) => map,
                Err(error) => {
                    eprintln!("{file}: does not parse: {error}");

                    return ExitCode::FAILURE;
                }
            };

            print!("{}", smap::to_mermaid_with(&map, layout));

            ExitCode::SUCCESS
        }
        Some("validate") => {
            let files = &arguments[2..];

            if files.is_empty() {
                eprintln!("usage: smap validate <map.json>…");

                return ExitCode::FAILURE;
            }

            let mut ok = true;

            for file in files {
                let text = match fs::read_to_string(file) {
                    Ok(text) => text,
                    Err(error) => {
                        eprintln!("{file}: cannot read: {error}");

                        ok = false;

                        continue;
                    }
                };

                let map = match smap::SystemMap::from_json(&text) {
                    Ok(map) => map,
                    Err(error) => {
                        eprintln!("{file}: does not parse: {error}");

                        ok = false;

                        continue;
                    }
                };

                match smap::validate(&map) {
                    Ok(()) => {
                        let targets = smap::SystemMap::discovery_targets(&map)
                            .iter()
                            .map(|function| function.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");

                        println!(
                            "{file}: {} — valid ({} elements, {} flows, {} functions; \
                             discovery targets: {}; hash {})",
                            map.name,
                            map.elements.len(),
                            map.flows.len(),
                            map.functions.len(),
                            if targets.is_empty() {
                                "none".to_string()
                            } else {
                                targets
                            },
                            smap::map_hash(&map),
                        );
                    }
                    Err(report) => {
                        eprintln!("{file}: invalid\n{report}");

                        ok = false;
                    }
                }
            }

            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        _ => {
            eprintln!("usage: smap <schema|validate <map.json>…|mermaid [--layout=lr|td] <map.json>>");

            ExitCode::FAILURE
        }
    }
}