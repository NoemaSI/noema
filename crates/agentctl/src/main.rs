use agentctl::handlers::GoalRun;
use agentctl::server::AgentServer;

const LISTEN: &str = "0.0.0.0:3333";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("serve") {
        eprintln!("usage: agentctl serve [addr]");
        std::process::exit(1);
    }
    let listen = args.get(2).map(String::as_str).unwrap_or(LISTEN);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(
        AgentServer::builder()
            .register::<GoalRun>()
            .serve(listen),
    );
}