use std::{fs::{ReadDir, read_dir}, path::PathBuf};

use noema_config::{NoemaConfig, NOEMA_CONFIG_SYSTEMMAP_FOLDER};

pub enum FunctionalDir {
    SystemMap
}

/// prints the file names of the NOEMA dir for a functional directory
pub fn print_functional_dir_content(work_dir: &PathBuf, functional_dir: FunctionalDir){
    // TODO: refactor to use read_functional_dir
    
    match functional_dir {
        FunctionalDir::SystemMap => {
            let path_on_disk = work_dir.join(NOEMA_CONFIG_SYSTEMMAP_FOLDER);
            if !path_on_disk.exists() {
                println!("{} doesnt exist", path_on_disk.display());
                return
            }
            let dir_content = read_dir(path_on_disk).expect("unable to read path");
            let mut dir_len = 0;
            for entry in dir_content {
                let entry = entry.expect("unable to read entry");
                dir_len +=1;
                if !entry.path().is_dir() {
                    println!("- {}", entry.file_name().display())
                }
            }
            if dir_len == 0 {
                println!("dir is empty");
            }
        }
    }
}

/// returns an iterator of the files of a functional directory in the NOEMA dir
pub fn read_functional_dir(work_dir: &PathBuf, functional_dir: FunctionalDir) -> ReadDir{
    match functional_dir {
        FunctionalDir::SystemMap => {
            let path_on_disk = work_dir.join(NOEMA_CONFIG_SYSTEMMAP_FOLDER);
            if !path_on_disk.exists() {
                println!("{} doesnt exist", path_on_disk.display());
            }
            return read_dir(path_on_disk).expect("unable to read path");
        }
    }
}

/// prints the content of the workdir to stdout
pub fn print_workdir_content(config: NoemaConfig) {
    let workdir = config.workdir;
    println!("showing content of workdir: {}", &workdir.display());
    println!("System maps:");
    println!("------------");
    print_functional_dir_content(&workdir, FunctionalDir::SystemMap);
}

