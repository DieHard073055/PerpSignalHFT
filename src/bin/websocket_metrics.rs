use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::{sync::mpsc, time};
use tokio_tungstenite::connect_async;
use url::Url;

#[derive(Debug, Deserialize)]
#[serde(tag = "e")]
pub enum StreamMessage {
    #[serde(rename = "trade")]
    Trade {
        E: u64,
        s: String,
        p: String,
        q: String,
        T: u64,
        m: bool,
    },
    #[serde(rename = "aggTrade")]
    AggTrade {
        E: u64,
        s: String,
        p: String,
        q: String,
        f: u64,
        l: u64,
        T: u64,
        m: bool,
    },
}

#[derive(Debug, Default)]
struct StreamStats {
    last_time: Option<Instant>,
    total_delta: Duration,
    count: u64,
    delta_count: u64,
    total_qty: f64
}

impl StreamStats {
    fn record(&mut self, qty: f64) {
        let now = Instant::now();
        if let Some(last) = self.last_time {
            let delta = now - last;
            self.total_delta += delta;
            self.delta_count += 1;
        }
        self.last_time = Some(now);
        self.count += 1;
        self.total_qty += qty;
    }

    fn reset_and_report(&mut self, label: &str) {
        println!("--- {label} ---");
        println!("Messages: {}", self.count);
        if self.delta_count > 0 {
            let avg = Duration::from_nanos(
                self.total_delta
                    .as_nanos()
                    .checked_div(self.delta_count as u128)
                    .unwrap_or(0) as u64,
            );
            println!("Avg time between messages: {:.2?}", avg);
        } else {
            println!("Avg time between messages: N/A");
        }

        if self.count > 0 {
            let avg_qty = self.total_qty / self.count as f64;
            println!("Avg qty per message: {:.6}", avg_qty);
        } else {
            println!("Avg qty per message: N/A");
        }

        self.count = 0;
        self.delta_count = 0;
        self.total_delta = Duration::ZERO;
        self.total_qty = 0.0;
        self.last_time = None;
    }
}

#[tokio::main]
async fn main() {
    let symbol = "btcusdt";
    let streams = format!("{}@trade/{}@aggTrade", symbol, symbol);
    let url = format!("wss://fstream.binance.com/stream?streams={}", streams);
    let (ws_stream, _) = connect_async(Url::parse(&url).unwrap())
        .await
        .expect("Failed to connect");
    println!("Connected to Binance WebSocket");

    let (mut write, mut read) = ws_stream.split();

    let mut stats: HashMap<String, StreamStats> = HashMap::new();
    let mut interval = time::interval(Duration::from_secs(60));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            msg = read.next() => {
                if let Some(Ok(msg)) = msg {
                    if let Ok(text) = msg.to_text() {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
                            if let Some(data) = value.get("data") {
                                match serde_json::from_value::<StreamMessage>(data.clone()) {
                                    Ok(StreamMessage::Trade { q, .. }) => {
                                        if let Ok(qty) = q.parse::<f64>() {
                                            stats.entry("trade".into()).or_default().record(qty);
                                        }
                                    }
                                    Ok(StreamMessage::AggTrade { q, .. }) => {
                                        if let Ok(qty) = q.parse::<f64>() {
                                            stats.entry("aggTrade".into()).or_default().record(qty);
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to parse stream message: {e:?}");
                                    }
                                }
                            }
                        }
                    }
                } else {
                    println!("WebSocket stream ended");
                    break;
                }
            }
            _ = interval.tick() => {
                for (stream_type, stat) in stats.iter_mut() {
                    stat.reset_and_report(stream_type);
                }
            }
        }
    }
}

