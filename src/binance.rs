use crate::format::Trade;
use futures::stream::{self, StreamExt};
use futures_util::SinkExt;
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer};
use std::future::Future;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum TradeMessageError {
    #[error("invalid message from websocket")]
    InvalidMessageFromWebsocket,
    #[error("unable to parse json message: {0}")]
    JsonParseError(#[from] serde_json::Error),
    #[error("failed to send pong")]
    FailedToSendPong,
}
#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum BinanceWebsocketError {
    #[error("Failed to send pong: {0}")]
    FailedToSendPong(String),
    #[error("web socket connection error: {0}")]
    WebsocketConnectionError(String),
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct TradeMessage {
    pub timestamp: u64,
    pub asset: String,
    pub price: String,
    pub quantity: String,
    pub is_buyer_maker: bool,
    pub received_at: u128,
}

impl TradeMessage {
    pub fn to_trade(self) -> Trade {
        let price: f64 = self.price.parse().unwrap();
        let quantity: f64 = self.quantity.parse().unwrap();
        Trade {
            timestamp: self.timestamp,
            symbol: self.asset,
            price,
            quantity,
            is_buyer_maker: self.is_buyer_maker,
        }
    }
    #[allow(dead_code)]
    pub fn create_from_ws(msg: Message) -> Result<Self, TradeMessageError> {
        let text = match msg {
            Message::Text(t) => t,
            _ => return Err(TradeMessageError::InvalidMessageFromWebsocket),
        };

        let json: serde_json::Value = serde_json::from_str(&text)?;

        let data = json.get("data").ok_or(TradeMessageError::JsonParseError(
            serde_json::Error::custom("missing `data` field"),
        ))?;

        let timestamp = // std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64;
        data
            .get("T")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| serde_json::Error::custom("`T` not u64"))?;

        let asset = data
            .get("s")
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde_json::Error::custom("`s` not str"))?
            .to_string();

        let price = data
            .get("p")
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde_json::Error::custom("`p` not str"))?
            .to_string();
        let quantity = data
            .get("q")
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde_json::Error::custom("`q` not str"))?
            .to_string();

        let is_buyer_maker = data
            .get("m")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| serde_json::Error::custom("`m` not bool"))?;

        Ok(Self {
            timestamp,
            asset,
            price,
            quantity,
            is_buyer_maker,
            received_at: std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_micros(),
        })
    }
}

/// Retry an async operation up to `max_retries` times, with exponential backoff.
///
/// - `op` is a zero-arg closure returning a Future that yields `Result<T, E>`.
/// - on `Ok(t)` we return `Ok(t)`.
/// - on `Err(e)` we wait `2.pow(attempt)` seconds and try again, up to `max_retries`,
///   after which we return the last `Err(e)`.
pub async fn retry_with_backoff<Op, Fut, T, E>(mut op: Op, max_retries: u32) -> Result<T, E>
where
    Op: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    let mut attempt = 0;
    loop {
        match op().await {
            Ok(val) => return Ok(val),
            Err(err) if attempt < max_retries => {
                attempt += 1;
                let backoff = tokio::time::Duration::from_secs(2u64.pow(attempt));
                tracing::warn!(
                    "operation failed (attempt #{}) – retrying in {:?}: {:?}",
                    attempt,
                    backoff,
                    err
                );
                tokio::time::sleep(backoff).await;
                // try again
            }
            Err(err) => {
                // out of retries
                return Err(err);
            }
        }
    }
}

pub struct BinanceWebsocket {
    pub assets: Vec<String>,
}
impl BinanceWebsocket {
    pub fn new(assets: Vec<String>) -> Self {
        Self { assets }
    }
    pub async fn start(
        &self,
        s: tokio::sync::mpsc::UnboundedSender<TradeMessage>,
    ) -> Result<(), BinanceWebsocketError> {
        let url = {
            let streams = self
                .assets
                .iter()
                .map(|asset| asset.to_lowercase() + "@trade")
                .collect::<Vec<String>>()
                .join("/");
            format!("wss://fstream.binance.com/stream?streams={}", streams)
        };

        tracing::info!("attempting to connect to {}", url);
        // wrap the async connect in a zero-arg closure
        let connect_op = || connect_async(&url);

        let (mut ws_stream, _) = retry_with_backoff(connect_op, 5)
            .await
            .map_err(|e| BinanceWebsocketError::WebsocketConnectionError(e.to_string()))?;

        tracing::info!("connected to the websocket!");
        while let Some(message) = ws_stream.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    match TradeMessage::create_from_ws(Message::Text(text)) {
                        Ok(msg) => {
                            let _ = s.send(msg);
                        }
                        Err(e) => tracing::warn!("bag message: {}", e),
                    }
                }
                Ok(Message::Ping(ping)) => {
                    // Respond to pings to keep connection alive
                    if let Err(e) = ws_stream.send(Message::Pong(ping)).await {
                        tracing::error!("Failed to send PONG: {}", e);
                        return Err(BinanceWebsocketError::FailedToSendPong(e.to_string()));
                    }
                }
                Err(e) => {
                    tracing::error!("WebSocket error: {}", e);
                    return Err(BinanceWebsocketError::WebsocketConnectionError(
                        e.to_string(),
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum BinanceError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("URL error: {0}")]
    Url(#[from] url::ParseError),

    #[error("Serde JSON error: {0}")]
    Serde(#[from] serde_json::Error),
}

// custom deserializer for string→f64
fn de_string_to_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse()
        .map_err(|e| D::Error::custom(format!("parse float error: {}", e)))
}

#[derive(Debug, Deserialize)]
struct RawTrade {
    #[serde(rename = "price", deserialize_with = "de_string_to_f64")]
    price: f64,

    #[serde(rename = "qty", deserialize_with = "de_string_to_f64")]
    qty: f64,
}

#[derive(Debug, Default)]
pub struct AvgPriceQty {
    pub price: f64,
    pub qty: f64,
}

#[derive(Clone)]
pub struct BinanceClient {
    http: reqwest::Client,
    base: url::Url,
}

impl Default for BinanceClient {
    fn default() -> Self {
        Self {
            http: reqwest::Client::new(),
            base: url::Url::parse("https://fapi.binance.com").unwrap(),
        }
    }
}
impl BinanceClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fetch recent trades for `symbol` and compute their average price & qty.
    pub async fn avg_stats(&self, symbol: String) -> Result<AvgPriceQty, BinanceError> {
        let url = self
            .base
            .join(&format!("/fapi/v1/trades?symbol={}", symbol))?;

        // GET … → Vec<RawTrade>
        let trades: Vec<RawTrade> = self.http.get(url).send().await?.json().await?;
        let n = trades.len() as f64;
        if n == 0.0 {
            return Ok(AvgPriceQty::default());
        }

        let (sum_p, sum_q) = trades
            .into_iter()
            .fold((0.0, 0.0), |(sp, sq), t| (sp + t.price, sq + t.qty));

        Ok(AvgPriceQty {
            price: sum_p / n,
            qty: sum_q / n,
        })
    }

    /// Compute averages for all symbols, up to `max_concurrency` at a time.
    pub async fn avg_stats_batch(
        &self,
        symbols: Vec<String>,
        max_concurrency: usize,
    ) -> Vec<AvgPriceQty> {
        let client = self.clone();
        stream::iter(symbols.into_iter()) // OWNED String
            .map(move |sym: String| {
                let cli = client.clone();
                async move { cli.avg_stats(sym).await.unwrap_or_default() }
            })
            .buffer_unordered(max_concurrency)
            .collect()
            .await
    }
}
