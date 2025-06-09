use perp_signal_hft::format::{BinaryFormat, BinaryFormatError, Trade};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Format error: {0}")]
    Format(#[from] BinaryFormatError),
    #[error("Time error: {0}")]
    Time(#[from] std::time::SystemTimeError),
}

fn handle_client(mut stream: TcpStream) -> Result<(), AppError> {
    stream.set_nodelay(true)?;

    // Sending a start hand shake
    let start = b"START";
    let len = (start.len() as u32).to_le_bytes();
    stream.write_all(&len)?;
    stream.write_all(start)?;

    let assets = vec![
        "BTCUSDT".to_string(),
        "ETHUSDT".to_string(),
        "SOLUSDT".to_string(),
    ];
    let mut encoder = BinaryFormat::new().with_assets(assets.clone())?;
    let reference_timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let reference_prices = vec![45000.0, 2500.5, 120.75];
    let reference_quantities = vec![0.0, 0.0, 0.0];
    let mut header_buf = Vec::new();
    encoder.write_header(
        &mut header_buf,
        reference_timestamp,
        &reference_prices,
        &reference_quantities,
    )?;
    let hdr_len = (header_buf.len() as u32).to_le_bytes();
    stream.write_all(&hdr_len)?;
    stream.write_all(&header_buf)?;

    for i in 0..10 {
        let idx = (i % assets.len()) as usize;
        let symbol = &assets[idx];
        let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        let price = reference_prices[idx] + (i as f64);
        let quantity = 0.01 * (i as f64 + 1.0);
        let is_buyer_maker = i % 2 == 0;
        let trade = Trade {
            symbol: symbol.clone(),
            timestamp: ts,
            price,
            quantity,
            is_buyer_maker,
        };
        let encoded = encoder.encode(&trade)?;
        let msg_len = (encoded.len() as u32).to_le_bytes();
        stream.write_all(&msg_len)?;
        stream.write_all(&encoded)?;

        println!("Server: sent {:?}", trade);
        sleep(Duration::from_millis(50));
    }

    Ok(())
}

fn main() -> Result<(), AppError> {
    let listener = TcpListener::bind("0.0.0.0:9000")?;
    println!("Server listening on port 9000");
    for stream in listener.incoming().take(1) {
        handle_client(stream?)?;
    }
    Ok(())
}
