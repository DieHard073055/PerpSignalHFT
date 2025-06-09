// src/tcp_server.rs
use std::net::SocketAddr;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

/// Runs the TCP server: listens, handshakes each client, and then streams messages.
pub async fn serve(
    bind_addr: &str,
    header: Vec<u8>,
    broadcaster: broadcast::Sender<Vec<u8>>,
) -> Result<(), std::io::Error> {
    let listener = TcpListener::bind(bind_addr).await?;
    tracing::info!("TCP server listening on {}", bind_addr);

    loop {
        let (socket, peer) = listener.accept().await?;
        tracing::info!("New client: {}", peer);

        let header = header.clone();
        let broadcaster_clone = broadcaster.clone();
        tokio::spawn(async move {
            if let Err(e) = handshake_and_serve(socket, peer, header, broadcaster_clone).await {
                tracing::error!("client {} error: {}", peer, e);
            }
            tracing::info!("client {} disconnected", peer);
        });
    }
}

async fn handshake_and_serve(
    mut socket: tokio::net::TcpStream,
    peer: SocketAddr,
    header: Vec<u8>,
    broadcaster: broadcast::Sender<Vec<u8>>,
) -> Result<(), std::io::Error> {
    let start = b"START";
    socket
        .write_all(&(start.len() as u32).to_le_bytes())
        .await?;
    socket.write_all(start).await?;

    socket
        .write_all(&(header.len() as u32).to_le_bytes())
        .await?;
    socket.write_all(&header).await?;
    let mut sub = broadcaster.subscribe();

    loop {
        match sub.recv().await {
            Ok(msg) => {
                socket.write_all(&(msg.len() as u32).to_le_bytes()).await?;
                socket.write_all(&msg).await?;
 
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!("{} lagged by {} msgs", peer, skipped);
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }

    Ok(())
}
