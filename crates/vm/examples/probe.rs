use vm::Vm;

fn main() {
    let vm = Vm::attach("noema").expect("attach");
    println!("state: {:?}", vm.state());
    println!("running: {}", vm.is_running());
    println!("pid: {:?}", vm.pid());
    println!("guest_ports: {:?}", vm.inner().guest_ports());
    println!("host_port(3333): {:?}", vm.host_port(3333).unwrap());
    println!("host_port(22): {:?}", vm.host_port(22).unwrap());
}