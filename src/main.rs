use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::mpsc;

use flintcore::config::Config;
use flintcore::net::{listener, NetEvent};
use flintcore::server::Server;
use flintcore::util::{log, Result};

fn parse_args() -> PathBuf {
    let mut config = PathBuf::from("flintcore.toml");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            if let Some(path) = args.next() {
                config = PathBuf::from(path);
            }
        }
    }
    config
}

fn main() -> Result<()> {
    let config_path = parse_args();
    let config = Config::load_or_default(&config_path)?;
    log::info(&format!(
        "flintcore starting with {}",
        config_path.display()
    ));

    let running = Arc::new(AtomicBool::new(true));
    let next_id = Arc::new(AtomicU64::new(1));
    let (inbound_tx, inbound_rx) = mpsc::channel::<NetEvent>(config.network.inbound_capacity);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let address = format!("{}:{}", config.server.bind, config.server.port);
    let listener = runtime.block_on(TcpListener::bind(&address))?;
    log::info(&format!("listening on {address}"));

    let listener_config = Arc::new(config.clone());
    runtime.spawn(listener::run(
        listener,
        inbound_tx,
        Arc::clone(&next_id),
        listener_config,
        Arc::clone(&running),
    ));

    {
        let running = Arc::clone(&running);
        runtime.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                log::info("shutdown signal received");
                running.store(false, Ordering::Relaxed);
            }
        });
    }

    let server = Server::new(config, inbound_rx, Arc::clone(&running));
    let result = server.run();
    runtime.shutdown_timeout(Duration::from_secs(2));
    result
}
