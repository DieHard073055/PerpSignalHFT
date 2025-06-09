use perp_signal_hft::format::{BinaryFormat, BinaryFormatError, Trade};
use rand::{Rng, rng};
use std::io::Cursor;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn main() -> Result<(), BinaryFormatError> {
    let assets = vec![
        "BTCUSDT".to_string(),
        "ETHUSDT".to_string(),
        "SOLUSDT".to_string(),
    ];
    let mut encoder = BinaryFormat::new().with_assets(assets.clone())?;
    let mut decoder = BinaryFormat::new().with_assets(assets.clone())?;

    // Write header into a buffer and have the decoder read it
    let reference_timestamp = 1_700_000_000_000;
    let reference_prices = vec![45_000.0, 2_500.5, 120.75];
    let reference_quantities = vec![0.0, 0.0, 0.0];

    let mut header_buf = Vec::new();
    encoder.write_header(
        &mut header_buf,
        reference_timestamp,
        &reference_prices,
        &reference_quantities,
    )?;
    // decoder consumes the header and populates its symbol → index map
    decoder.read_header(&mut Cursor::new(&header_buf))?;

    let mut rng = rng();
    loop {
        // generate a random Trade
        let idx = rng.random_range(0..assets.len());
        let symbol = assets[idx].clone();
        let ref_price = reference_prices[idx];
        let trade = Trade {
            symbol,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            price: ref_price + rng.random_range(-10.0..10.0),
            quantity: rng.random_range(0.001..1.0),
            is_buyer_maker: rng.random_bool(0.5),
        };

        // encode the trade
        let encoded = encoder.encode(&trade)?;

        // decode on a fresh cursor
        let decoded = decoder.read_message(&mut Cursor::new(&encoded))?;
        println!("Decoded trade: {:?}", decoded);

        sleep(Duration::from_millis(100));
    }
}
