use tokio::prelude::*;
use std::net::SocketAddr;
mod udp_runtime;
use udp_runtime::UdpRuntime;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_addr = SocketAddr::from(([0, 0, 0, 0], 1324));
    let (mut receiver, mut udp_runtime_rx, mut sender, mut udp_runtime_tx) = UdpRuntime::new(socket_addr).await?;
    tokio::spawn(async move {
        udp_runtime_rx.run().await.unwrap();
    });

    tokio::spawn(async move {
        udp_runtime_tx.run().await.unwrap();
    });



    Ok(())
}
