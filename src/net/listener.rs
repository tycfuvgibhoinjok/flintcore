use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::net::connection;
use crate::net::NetEvent;
use crate::util::log;

pub async fn run(
    listener: TcpListener,
    inbound: mpsc::Sender<NetEvent>,
    next_id: Arc<AtomicU64>,
    config: Arc<Config>,
    running: Arc<AtomicBool>,
) {
    while running.load(Ordering::Relaxed) {
        match listener.accept().await {
            Ok((stream, address)) => {
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                let (sender, receiver) = mpsc::channel(config.network.outbound_capacity);
                if inbound
                    .send(NetEvent::Connected {
                        id,
                        address,
                        sender,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
                let connection_inbound = inbound.clone();
                let connection_config = Arc::clone(&config);
                tokio::spawn(connection::serve(
                    id,
                    stream,
                    connection_inbound,
                    receiver,
                    connection_config,
                ));
            }
            Err(error) => log::warn(&format!("accept failed: {error}")),
        }
    }
}
