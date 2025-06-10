// std
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// external
use clap::Parser;
use tokio::sync::{broadcast, mpsc::UnboundedReceiver};

// internal
use perp_signal_hft::binance::{BinanceClient, BinanceError, BinanceWebsocket, TradeMessage};
use perp_signal_hft::cli::Cli;
use perp_signal_hft::format::{BinaryFormat, BinaryFormatError};
use perp_signal_hft::ipc::shm_queue::ShmQueue;
use perp_signal_hft::ipc::tcp;

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
    tracing::info!(
        "Initializing encoder for {} assets: {:?}",
        assets.len(),
        assets
    );

    let asset_len = assets.len();

    tracing::debug!("Fetching price/quantity stats from Binance");
    let pnqs = BinanceClient::new()
        .avg_stats_batch(assets.clone(), asset_len)
        .await;

    tracing::debug!("Received {} price/qty pairs from Binance", pnqs.len());
    let mut prices = Vec::with_capacity(pnqs.len());
    let mut qtys = Vec::with_capacity(pnqs.len());
    for pnq in pnqs {
        prices.push(pnq.price);
        qtys.push(pnq.qty);
    }
    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let mut encoder = BinaryFormat::new().with_assets(assets)?;
    let mut header = Vec::new();
    encoder.write_header(&mut header, ts, &prices, &qtys)?;
    tracing::info!(
        "Encoder initialized successfully with {} byte header",
        header.len()
    );
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
    tracing::info!("Starting trade processing pipeline");
    callback(b"START".to_vec()).await;
    callback(header.clone()).await;
    tracing::info!("Header sent, waiting for trades");
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
    tracing::info!(
        "Setting up SHM queue: name='{}', capacity={} bytes",
        name,
        capacity
    );
    let queue = Arc::new(ShmQueue::create(&name, capacity)?);
    tracing::info!("SHM queue created successfully");
    let (encoder, header) = initialize_encoder(assets).await?;

    let callback = {
        move |data: Vec<u8>| {
            let queue = queue.clone();
            async move {
                if let Err(e) = queue.push(&data) {
                    tracing::error!("SHM push failed: {}", e);
                }
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
    tracing::info!("Setting up TCP server on {}", bind_addr);
    let (encoder, header) = initialize_encoder(assets).await?;

    let (tx, _) = broadcast::channel::<Vec<u8>>(100);

    let tx_clone = tx.clone();
    let header_clone = header.clone();
    tokio::spawn(async move {
        handle_trades(encoder, header_clone, rx, move |data| {
            let _ = tx_clone.send(data);
            async {}
        })
        .await;
    });

    tracing::info!("Starting TCP server");
    tcp::serve(&bind_addr, header, tx).await?;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    tracing::info!("🚀 Starting perp_signal_hft");

    let cli = Cli::parse();

    if cli.assets.len() > 10 {
        tracing::error!("Too many assets: {} (max 10)", cli.assets.len());
        std::process::exit(1);
    }
    tracing::info!(
        "Configuration: assets={:?}, comm={:?}",
        cli.assets,
        cli.comm
    );

    let assets = cli.assets;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    tracing::info!("Starting Binance WebSocket connection");
    let assets_clone = assets.clone();
    let b_handle = tokio::spawn(async move {
        BinanceWebsocket::start(tx, &assets_clone)
            .await
            .expect("websocket failed");
    });

    let comm_type = match &cli.comm {
        perp_signal_hft::cli::Comm::Shm { name, .. } => format!("SHM ({})", name),
        perp_signal_hft::cli::Comm::Tcp { port } => format!("TCP (port {})", port),
    };
    tracing::info!("Using {} communication method", comm_type);

    let t_handle = match cli.comm {
        perp_signal_hft::cli::Comm::Shm { name, capacity } => tokio::spawn(async move {
            handle_trades_shm(assets, name, capacity, rx)
                .await
                .expect("SHM handler failed");
        }),
        perp_signal_hft::cli::Comm::Tcp { port } => {
            let bind_address = format!("0.0.0.0:{}", port);
            tokio::spawn(async move {
                handle_trades_tcp(assets, bind_address, rx)
                    .await
                    .expect("TCP handler failed");
            })
        }
    };

    tracing::info!("All components started, processing trades...");

    let (b_res, t_res) = tokio::join!(b_handle, t_handle);
    if let Err(e) = b_res {
        tracing::error!("binance websocket handle panicked {}", e);
    }
    t_res.expect("trade signal handler panicked");
}
