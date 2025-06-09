// consumer.rs
use perp_signal_hft::{
    format::{BinaryFormat, Trade},
    ipc::shm_queue::ShmQueue,
};
use std::{
    hint,
    io::Cursor,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Shared memory queue must match producer
    let capacity = 1024 * 1024;
    let queue_name = "trade_queue";

    // Init decoder and SHM queue
    let mut decoder = BinaryFormat::new();
    let queue = ShmQueue::create(queue_name, capacity)?;

    // 1️⃣ Wait for START handshake
    loop {
        if let Some(data) = queue.pop()? {
            if data == b"START" {
                println!("Consumer: received START handshake");
                break;
            }
        }
        hint::spin_loop();
    }

    // 2️⃣ Read header
    let header_buf = loop {
        if let Some(buf) = queue.pop()? {
            break buf;
        }
        hint::spin_loop();
    };
    decoder.read_header(&mut Cursor::new(&header_buf))?;
    println!("Consumer: read HEADER");

    // 3️⃣ Consume and decode 10 trades
    use std::time::Duration;
    let mut count = 0;
    loop {
        let data = loop {
            if let Some(buf) = queue.pop()? {
                break buf;
            }
            hint::spin_loop();
        };

        // Decode binary format
        let mut cursor = Cursor::new(&data);
        let trade: Trade = decoder.read_message(&mut cursor)?;

        // Calculate latency in nanoseconds
        let now_ns = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        let latency = now_ns.saturating_sub(trade.timestamp);
        count += 1;

        // Pretty print: ns with commas and ms with 3 decimal places
        println!(
            "Consumed {idx}: {trade:?}, latency {ms} millis",
            idx = count,
            trade = trade,
            ms = format_args!("{:}", latency),
        );
    }
}
