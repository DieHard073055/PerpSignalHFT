use perp_signal_hft::binance::{BinanceWebsocket, TradeMessage};
use tokio::time::{self, Duration};

pub async fn print_messages(mut r: tokio::sync::mpsc::UnboundedReceiver<TradeMessage>) {
    let mut interval = time::interval(Duration::from_secs(60));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    let mut count = 0u64;
    loop {
        tokio::select! {
            Some(message) = r.recv() => {
                count += 1;
                tracing::debug!("{:?}", message);
                tracing::info!("Messages in the last minute: {}", count);
            }

            _ = interval.tick() => {
                count = 0;
            }
        }
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(false)
        .init();
    tracing::info!("starting binance websocket executor");
    let assets = vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()];
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    let ws_handle = tokio::spawn(async move { BinanceWebsocket::start(tx, &assets).await });
    let print_handle = tokio::spawn(async move { print_messages(rx).await });

    let (ws_res, print_res) = tokio::join!(ws_handle, print_handle);

    let _ = ws_res.expect("websocket task panicked");
    print_res.expect("printer task panicked");
}
