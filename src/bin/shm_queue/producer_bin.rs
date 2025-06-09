use perp_signal_hft::binance::TradeMessage;
use perp_signal_hft::{
    format::{BinaryFormat, Trade},
    ipc::shm_queue::ShmQueue,
};
use std::{
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Shared memory queue parameters
    let capacity = 1024 * 1024; // 1 MiB
    let queue_name = "trade_queue";

    // Asset list and encoder
    let assets = vec![
        "BTCUSDT".to_string(),
        "ETHUSDT".to_string(),
        "SOLUSDT".to_string(),
    ];
    let mut encoder = BinaryFormat::new().with_assets(assets.clone())?;

    // Open/create SHM queue
    let queue = ShmQueue::create(queue_name, capacity)?;

    // 1️⃣ Send START handshake
    queue.push(b"START")?;
    println!("Producer: sent START handshake");

    // 2️⃣ Send header once
    let reference_timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let reference_prices = vec![45000.0f64, 2500.5f64, 120.75f64];
    let reference_quantities = vec![0.0f64, 0.0f64, 0.0f64];
    let mut header_buf = Vec::new();
    encoder.write_header(
        &mut header_buf,
        reference_timestamp,
        &reference_prices,
        &reference_quantities,
    )?;
    queue.push(&header_buf)?;
    println!("Producer: sent HEADER");

    // 3️⃣ Generate and send 10 sample trades
    for i in 0..100 {
        let idx = (i % assets.len()) as usize;
        let symbol = assets[idx].clone();
        // timestamp in µs
        let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros() as u64;
        let price = reference_prices[idx] + (i as f64);
        let quantity = 0.01 * (i as f64 + 1.0);
        let is_buyer_maker = i % 2 == 0;

        // let trade = Trade { symbol, timestamp: ts, price, quantity, is_buyer_maker };
        let b = TradeMessage {
            timestamp: ts,
            asset: symbol.clone(),
            price: price.to_string(),
            quantity: quantity.to_string(),
            is_buyer_maker,
            received_at: ts as u128,
        };
        let trade = b.to_trade();
        let encoded = encoder.encode(&trade)?;
        queue.push(&encoded)?;
        println!("Produced {}: {:?}", i, trade);

        thread::sleep(Duration::from_millis(50));
    }

    Ok(())
}
