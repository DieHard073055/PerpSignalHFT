use clap::Parser;
use perp_signal_hft::binance::{BinanceClient, BinanceError, BinanceWebsocket, TradeMessage};
use perp_signal_hft::cli::Cli;
use perp_signal_hft::format::{BinaryFormat, BinaryFormatError};
use perp_signal_hft::ipc::shm_queue::ShmQueue;
use perp_signal_hft::ipc::tcp;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc::UnboundedReceiver};


#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("BinanceTrade error: {0}")]
    BinanceError(#[from] BinanceError),
    #[error("Format error: {0}")]
    Format(#[from] BinaryFormatError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Time error: {0}")]
    Time(#[from] std::time::SystemTimeError),
}

async fn initialize_encoder(assets: Vec<String>) -> Result<(BinaryFormat, Vec<u8>), PipelineError> {
    // Fetch reference AvgPriceQty for each asset
    let pnqs = BinanceClient::new()
        .avg_stats_batch(assets.clone(), assets.len())
        .await;
    let mut prices = Vec::with_capacity(pnqs.len());
    let mut qtys = Vec::with_capacity(pnqs.len());
    for pnq in pnqs {
        prices.push(pnq.price);
        qtys.push(pnq.qty);
    }
    // Reference timestamp in ms
    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    // Create encoder and write header
    let mut encoder = BinaryFormat::new().with_assets(assets)?;
    let mut header = Vec::new();
    encoder.write_header(&mut header, ts, &prices, &qtys)?;
    tracing::info!("encoder initialized");
    Ok((encoder, header))
}

/// Generic handler: applies `callback` to the header and every encoded trade.
async fn handle_trades<F, Fut>(
    mut encoder: BinaryFormat,
    header: Vec<u8>,
    mut rx: UnboundedReceiver<TradeMessage>,
    callback: F,
) where
    F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    callback(b"START".to_vec()).await;
    callback(header.clone()).await;
    tracing::info!("header sent!");
    while let Some(msg) = rx.recv().await {
        match encoder.encode(&msg.to_trade()) {
            Ok(bin) => callback(bin).await,
            Err(e) => tracing::error!("encode error: {}", e),
        }
    }
}

/// SHM-based pipeline: writes header and trades into shared memory queue.
pub async fn handle_trades_shm(
    assets: Vec<String>,
    name: String,
    capacity: u32,
    rx: UnboundedReceiver<TradeMessage>,
) -> Result<(), PipelineError> {
    let queue = Arc::new(ShmQueue::create(&name, capacity)?);
    let (encoder, header) = initialize_encoder(assets.clone()).await?;

    let queue_ref = queue.clone();
    let callback = move |data: Vec<u8>| {
        let q = queue_ref.clone();
        async move {
            if let Err(e) = q.push(&data) {
                tracing::error!("shm push failed: {}", e);
            }
        }
    };
    handle_trades(encoder, header, rx, callback).await;
    Ok(())
}

/// TCP-based pipeline: broadcasts START, header, and trades to all connected clients.
pub async fn handle_trades_tcp(
    assets: Vec<String>,
    bind_addr: String,
    rx: UnboundedReceiver<TradeMessage>,
) -> Result<(), PipelineError> {
    let (encoder, header) = initialize_encoder(assets.clone()).await?;

    let (tx, _) = broadcast::channel::<Vec<u8>>(100);

    let tx2 = tx.clone();
    let header_clone = header.clone();
    tokio::spawn(async move {
        handle_trades(encoder, header_clone, rx, move |data| {
            let _ = tx2.send(data);
            async {}
        })
        .await;
    });

    tcp::serve(&bind_addr, header, tx.clone()).await?;
    Ok(())
}


#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
pub async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    let cli = Cli::parse();

    if cli.assets.len() > 10 {
        tracing::error!("You cant have more than 10 assets.");
        std::process::exit(1);
    }
    tracing::info!(
        "Configuration: assets={:?}, comm={:?}",
        cli.assets,
        cli.comm
    );

    let assets = cli.assets.clone();
    let b = BinanceWebsocket::new(assets.clone());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();


    let b_handle = tokio::spawn(async move {
        b.start(tx).await.expect("websocket failed");
    });
    let t_handle = match cli.comm {
        perp_signal_hft::cli::Comm::Shm { name, capacity } => {
            tokio::spawn(async move {
                handle_trades_shm(assets, name, capacity, rx)
                    .await
                    .expect("SHM handler failed");
            })
        }
        perp_signal_hft::cli::Comm::Tcp { port } => {
            let bind_address = format!("0.0.0.0:{}", port); // String
            tokio::spawn(async move {
                handle_trades_tcp(assets, bind_address, rx)
                    .await
                    .expect("TCP handler failed");
            })
        }
    };
    let (b_res, t_res) = tokio::join!(b_handle, t_handle);
    if let Err(e) = b_res {
        tracing::error!("binance websocket handle panicked {}", e);
    }
    t_res.expect("trade signal handler panicked");
}
