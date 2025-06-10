// consumer.rs
use clap::Parser;
use perp_signal_hft::{
    format::{BinaryFormat, Trade},
    ipc::shm_queue::ShmQueue,
};
use std::{
    hint,
    io::Cursor,
    time::{SystemTime, UNIX_EPOCH},
};

/// Simple SHM Consumer
#[derive(Parser)]
#[clap(name = "shm_consumer", about = "Read trades from SHM queue")]
struct Opts {
    /// SHM queue name
    #[clap(long, default_value = "trade_queue")]
    queue_name: String,

    /// Ring-buffer capacity in bytes
    #[clap(long, default_value_t = 1024 * 1024)]
    capacity: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opts = Opts::parse();
    let queue_name = &opts.queue_name;
    let capacity = opts.capacity;

    // Init decoder and SHM queue
    let mut decoder = BinaryFormat::new();
    let queue = ShmQueue::create(queue_name, capacity)?;

    loop {
        if let Some(data) = queue.pop()? {
            if data == b"START" {
                println!("Consumer: received START handshake");
                break;
            }
        }
        hint::spin_loop();
    }

    let header_buf = loop {
        if let Some(buf) = queue.pop()? {
            break buf;
        }
        hint::spin_loop();
    };
    decoder.read_header(&mut Cursor::new(&header_buf))?;
    println!("Consumer: read HEADER");

    let mut count = 0;
    loop {
        let data = loop {
            if let Some(buf) = queue.pop()? {
                break buf;
            }
            hint::spin_loop();
        };

        let mut cursor = Cursor::new(&data);
        let trade: Trade = decoder.read_message(&mut cursor)?;

        let now_ns = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        let latency = now_ns.saturating_sub(trade.timestamp);
        count += 1;

        println!(
            "Consumed {idx}: {trade:?}, latency {ms} millis",
            idx = count,
            trade = trade,
            ms = format_args!("{:}", latency),
        );
    }
}
