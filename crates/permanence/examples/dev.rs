use noema_config::NoemaConfig;
use permanence::print_workdir_content;

pub fn main (){
    let cfg = NoemaConfig::from_home().expect("unable to read noema_config");
    print_workdir_content(cfg);
}
