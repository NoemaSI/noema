use noema_config::NoemaConfig;
use permanence::{FunctionalDir, read_functional_dir};
use smap::{SystemMap, read_system_map, validate};

pub fn read_system_maps(cfg: &NoemaConfig) -> Vec<SystemMap>{
    let work_dir = &cfg.workdir;
    //print_workdir_content(cfg.clone());
    let mut valid_maps = vec![];
    let mut invalid_maps = vec![];

    // read and validate system maps
    for entry in read_functional_dir(&work_dir, FunctionalDir::SystemMap) {
        let entry = entry.expect("unable to read");
        println!("{}", entry.file_name().display());
        match read_system_map(entry.path()){
            Ok(map) => {
                match  validate(&map) {
                    Ok(_)  => valid_maps.push(map),
                    Err(e) => invalid_maps.push((e, entry.path(), map)),
                }
            }
            Err(e) => {
                eprint!("error: {}", e);
            }
            
        }
    }

    // TODO: show errors in ui
    
    // if we found invalid maps, we print their validation error
    if !invalid_maps.is_empty() {
        println!("these system maps contain validation errors and have not been processed:");
        for invalid_map in invalid_maps {
            println!("Validation error: {}", invalid_map.0);
            println!("for map: {}", invalid_map.1.display());
        } 

    }
    valid_maps
}
