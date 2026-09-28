mod handlers;

use agentctl::server::AgentServer;

const LISTEN: &str = "0.0.0.0:3333";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("serve") {
        eprintln!("usage: agentctl serve");
        std::process::exit(1);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(
        AgentServer::builder()
            .register::<handlers::GoalRun>()
            .serve(LISTEN),
    );
}