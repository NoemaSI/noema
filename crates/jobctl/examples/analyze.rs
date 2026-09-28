//! Headless reproduction of the Analyze click: prints every engine event.
use jobctl::jobs::intent::Intent;
use jobctl::engine;

fn main() {
    let (handle, mut results) = engine()
        .register::<Intent>()
        .queue_capacity(32)
        .worker_threads(1)
        .spawn();

    println!("catalogue: {:?}", handle.catalog());
    let id = handle
        .analyze_intent("test".into(), "characterize Kd".into())
        .expect("submit");
    println!("submitted root {id}");

    let rt_pump = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            while let Some(ev) = results.recv().await {
                println!("[{}] {:?} {}", ev.kind, ev.status, ev.payload);
                if ev.kind == "intent" && ev.status != jobctl::EventStatus::Progress {
                    break;
                }
            }
        });
    });
    rt_pump.join().unwrap();
}