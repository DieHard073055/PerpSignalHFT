use std::collections::HashMap;
use std::io::{Cursor, Read, Write};


const SCALE_FACTOR: f64 = 100000.0;

#[derive(Debug, thiserror::Error)]
pub enum BinaryFormatError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Invalid asset ID: {0}")]
    InvalidAssetId(String),

    #[error("Invalid symbol: {0}")]
    InvalidSymbol(String),

    #[error("Invalid version: {0}")]
    InvalidVersion(u8),

    #[error("Invalid header length")]
    InvalidHeaderLength,

    #[error("Insufficient data")]
    InsufficientData,

    #[error("Too many assets (max 127)")]
    TooManyAssets,

    #[error("Overflow error")]
    Overflow,
}

/// variable length integer encoding/decoding
pub mod varint {
    use super::*;

    pub fn encode_unsigned(
        value: u64,
        writer: &mut impl Write,
    ) -> Result<usize, BinaryFormatError> {
        let mut value = value;
        let mut bytes_written = 0;

        loop {
            let mut byte = (value & 0x7F) as u8; // Lower 7 bits
            value >>= 7; // Remaining bits
            if value != 0 {
                byte |= 0x80; // Set continuation bit
            }
            writer.write_all(&[byte])?;
            bytes_written += 1;
            if value == 0 {
                break;
            }
        }

        Ok(bytes_written)
    }

    pub fn decode_unsigned(reader: &mut impl Read) -> Result<u64, BinaryFormatError> {
        let mut result = 0u64;
        let mut shift = 0;

        loop {
            let mut byte = [0u8];
            reader.read_exact(&mut byte)?;
            let value = (byte[0] & 0x7F) as u64;
            result |= value << shift;

            if byte[0] & 0x80 == 0 {
                break;
            }

            shift += 7;
            if shift >= 64 {
                return Err(BinaryFormatError::InsufficientData);
            }
        }

        Ok(result)
    }

    pub fn encode_signed(value: i64, writer: &mut impl Write) -> Result<usize, BinaryFormatError> {
        // Apply zigzag encoding
        let encoded: u64 = ((value << 1) ^ (value >> 63)) as u64;
        encode_unsigned(encoded, writer)
    }

    pub fn decode_signed(reader: &mut impl Read) -> Result<i64, BinaryFormatError> {
        let encoded = decode_unsigned(reader)?;
        // apply zigzag decoding:
        // - Positive: Encoded value >> 1
        // - Negative: -(Encoded value >> 1) - 1
        Ok((encoded >> 1) as i64 ^ -((encoded & 1) as i64))
    }
}

#[derive(Debug, Clone)]
pub struct Trade {
    pub symbol: String,
    pub timestamp: u64,       // Timestamp in milliseconds
    pub price: f64,           // Trade price
    pub quantity: f64,        // Trade quantity
    pub is_buyer_maker: bool, // True for buyer maker, false otherwise
}

/// Header information for the binary format
#[allow(dead_code)]
#[derive(Debug)]
struct Header {
    version: u8,
    assets: Vec<String>,
    reference_timestamp: u64,
    reference_prices: Vec<f64>,
    reference_quantities: Vec<f64>,
}

/// State tracking for delta encoding
#[derive(Clone, Debug)]
struct AssetState {
    last_timestamp: u64,
    last_price: f64,
    last_quantity: f64,
}

/// Binary format encoder/decoder for trade data
pub struct BinaryFormat {
    version: u8,
    assets: Vec<String>,
    asset_to_id: HashMap<String, u8>,
    states: Vec<AssetState>,
}

impl Default for BinaryFormat {
    fn default() -> Self {
        let asset_to_id = HashMap::new();

        BinaryFormat {
            version: 1,
            assets: vec![],
            asset_to_id,
            states: Vec::new(),
        }
    }
}
impl BinaryFormat {
    pub fn new() -> Self {
        BinaryFormat::default()
    }
    pub fn with_assets(mut self, assets: Vec<String>) -> Result<Self, BinaryFormatError> {
        let asset_len = assets.len();
        if asset_len > 127 {
            return Err(BinaryFormatError::TooManyAssets);
        }

        let mut asset_to_id = HashMap::new();
        for (idx, asset) in assets.iter().enumerate() {
            asset_to_id.insert(asset.clone(), idx as u8);
        }

        self.assets = assets;
        self.asset_to_id = asset_to_id;
        self.states = vec![
            AssetState {
                last_timestamp: 0,
                last_price: 0.0,
                last_quantity: 0.0,
            };
            asset_len
        ];
        Ok(self)
    }

    pub fn write_header(
        &mut self,
        buffer: &mut Vec<u8>,
        reference_timestamp: u64,
        reference_prices: &[f64],
        reference_quantities: &[f64],
    ) -> Result<(), BinaryFormatError> {
        buffer.write_all(&[self.version])?;
        buffer.write_all(&[self.assets.len() as u8])?;

        for asset in &self.assets {
            buffer.write_all(&[asset.len() as u8])?;
            buffer.write_all(asset.as_bytes())?;
        }

        buffer.write_all(&reference_timestamp.to_le_bytes())?;

        for price in reference_prices {
            buffer.write_all(&price.to_le_bytes())?;
        }

        for qty in reference_quantities {
            buffer.write_all(&qty.to_le_bytes())?;
        }

        self.states = reference_prices
            .iter()
            .zip(reference_quantities)
            .map(|(p, q)| AssetState {
                last_timestamp: reference_timestamp,
                last_price: *p,
                last_quantity: *q,
            })
            .collect();

        Ok(())
    }

    pub fn read_header(&mut self, cursor: &mut Cursor<&Vec<u8>>) -> Result<(), BinaryFormatError> {
        let mut version = [0u8];
        cursor.read_exact(&mut version)?;
        if version[0] != self.version {
            return Err(BinaryFormatError::InvalidVersion(version[0]));
        }

        let mut asset_count = [0u8];
        cursor.read_exact(&mut asset_count)?;
        let asset_count = asset_count[0] as usize;

        let mut assets = Vec::with_capacity(asset_count);
        for _ in 0..asset_count {
            let mut symbol_len = [0u8];
            cursor.read_exact(&mut symbol_len)?;

            let mut symbol_bytes = vec![0u8; symbol_len[0] as usize];
            cursor.read_exact(&mut symbol_bytes)?;
            let symbol = String::from_utf8(symbol_bytes)
                .map_err(|_| BinaryFormatError::InvalidSymbol("Invalid UTF-8".to_string()))?;
            assets.push(symbol);
        }

        let mut ref_timestamp = [0u8; 8];
        cursor.read_exact(&mut ref_timestamp)?;
        let reference_timestamp = u64::from_le_bytes(ref_timestamp);

        let mut reference_prices = Vec::with_capacity(asset_count);
        for _ in 0..asset_count {
            let mut price_bytes = [0u8; 8];
            cursor.read_exact(&mut price_bytes)?;
            reference_prices.push(f64::from_le_bytes(price_bytes));
        }

        let mut reference_quantities = Vec::with_capacity(asset_count);
        for _ in 0..asset_count {
            let mut qty_bytes = [0u8; 8];
            cursor.read_exact(&mut qty_bytes)?;
            reference_quantities.push(f64::from_le_bytes(qty_bytes));
        }

        // Initialize the states and assets
        self.assets = assets;
        self.states = reference_prices
            .iter()
            .zip(reference_quantities.iter())
            .map(|(&price, &qty)| AssetState {
                last_timestamp: reference_timestamp,
                last_price: price,
                last_quantity: qty,
            })
            .collect();
        Ok(())
    }

    pub fn encode(&mut self, trade: &Trade) -> Result<Vec<u8>, BinaryFormatError> {
        let mut buffer = Vec::with_capacity(64);
        // Why did i set it to 64?
        // 
        // Symbol:
        // Maximum of 32 bytes (including UTF-8 data and length byte, if the symbol length is up to 31 characters).
        // Timestamp:
        // Varint (worst case): 10 bytes.
        // Price Delta:
        // Varint (worst case): 10 bytes.
        // Quantity:
        // Varint (worst case): 10 bytes.
        // Total Size = 32 + 10 + 10 + 10  = 62 bytes.

        self.write_message(trade, &mut buffer)?;
        Ok(buffer)
    }

    pub fn decode(&mut self, data: &Vec<u8>) -> Result<Trade, BinaryFormatError> {
        let mut cursor = Cursor::new(data);
        self.read_message(&mut cursor)
    }

    pub fn write_message(
        &mut self,
        trade: &Trade,
        buffer: &mut Vec<u8>,
    ) -> Result<(), BinaryFormatError> {
        let asset_id = *self
            .asset_to_id
            .get(&trade.symbol)
            .ok_or_else(|| BinaryFormatError::InvalidSymbol(trade.symbol.clone()))?;

        let packed_byte = if trade.is_buyer_maker {
            asset_id | 0x80
        } else {
            asset_id & 0x7F
        };

        buffer.write_all(&[packed_byte])?;

        let state = &mut self.states[asset_id as usize & 0x7F];

        let ts_delta = (trade.timestamp as i64)
            .checked_sub(state.last_timestamp as i64)
            .ok_or(BinaryFormatError::Overflow)?;

        varint::encode_signed(ts_delta, buffer)?;

        let price_delta = ((trade.price - state.last_price) * SCALE_FACTOR) as i64;
        varint::encode_signed(price_delta, buffer)?;

        let qty_fixed = (trade.quantity * SCALE_FACTOR) as u64;
        varint::encode_unsigned(qty_fixed, buffer)?;

        state.last_timestamp = trade.timestamp;
        state.last_price = trade.price;
        state.last_quantity = trade.quantity;

        Ok(())
    }

    pub fn read_message(
        &mut self,
        cursor: &mut Cursor<&Vec<u8>>,
    ) -> Result<Trade, BinaryFormatError> {
        let mut packed_byte = [0u8];
        cursor.read_exact(&mut packed_byte)?;
        let packed_byte = packed_byte[0];

        let is_buyer_maker = packed_byte & 0x80 != 0;
        let asset_id = packed_byte & 0x7F;

        if asset_id as usize >= self.assets.len() {
            return Err(BinaryFormatError::InvalidAssetId(format!(
                "Asset ID {} out of bounds (0 <= ID < {})",
                asset_id,
                self.assets.len()
            )));
        }
        let state = &mut self.states[asset_id as usize];

        let ts_delta = varint::decode_signed(cursor)?;
        let timestamp = ((state.last_timestamp as i64) + ts_delta) as u64;

        let price_delta = varint::decode_signed(cursor)?;
        let price = state.last_price + (price_delta as f64 / SCALE_FACTOR);

        let qty_fixed = varint::decode_unsigned(cursor)?;
        let quantity = qty_fixed as f64 / SCALE_FACTOR;

        state.last_timestamp = timestamp;
        state.last_price = price;
        state.last_quantity = quantity;

        Ok(Trade {
            symbol: self.assets[asset_id as usize].clone(),
            timestamp,
            price,
            quantity,
            is_buyer_maker,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint_encoding_decoding() {
        let mut buffer = Vec::new();

        // Values that are powers of 2 and edge cases
        let unsigned_test_values = vec![
            0u64,
            127u64,     // 1-byte limit
            128u64,     // 2-byte transition point
            16384u64,   // 3-byte transition point
            2097151u64, // 4-byte transition
            u64::MAX,   // Max unsigned value (64 bits)
        ];

        for value in unsigned_test_values {
            buffer.clear();

            // Encode the unsigned value
            varint::encode_unsigned(value, &mut buffer).unwrap();

            // Decode the unsigned value
            let decoded = varint::decode_unsigned(&mut Cursor::new(&buffer)).unwrap();

            // Assert the original matches the decoded value
            assert_eq!(
                decoded, value,
                "Unsigned varint failed for value: {}",
                value
            );
        }

        // Assert specific binary encodings for some unsigned values
        buffer.clear();
        varint::encode_unsigned(127, &mut buffer).unwrap();
        assert_eq!(buffer, vec![0x7F]); // Single byte for 127

        buffer.clear();
        varint::encode_unsigned(128, &mut buffer).unwrap();
        assert_eq!(buffer, vec![0x80, 0x01]); // Two bytes for 128

        buffer.clear();
        varint::encode_unsigned(300, &mut buffer).unwrap();
        assert_eq!(buffer, vec![0xAC, 0x02]); // Two bytes for 300

        // --- Test signed varint encoding/decoding ---
        // Test values including edge cases, negatives, and zero
        let signed_test_values = vec![
            0i64,
            1i64,
            -1i64,    // Smallest negative
            63i64,    // Zigzag-encoded as 126
            -64i64,   // Zigzag-encoded as 127
            64i64,    // Zigzag-encoded as 128 (2-byte transition)
            -64i64,   // Zigzag-encoded as 129
            1023i64,  // Zigzag-encoded as 2046
            -1024i64, // Zigzag-encoded as 2047
            i64::MAX, // Maximum possible i64
            i64::MIN, // Most negative i64 (edge case)
        ];

        for value in signed_test_values {
            buffer.clear();

            // Encode the signed value
            varint::encode_signed(value, &mut buffer).unwrap();

            // Decode the signed value
            let decoded = varint::decode_signed(&mut Cursor::new(&buffer)).unwrap();

            // Assert the original matches the decoded value
            assert_eq!(decoded, value, "Signed varint failed for value: {}", value);
        }

        // Assert specific binary encodings for some signed values
        buffer.clear();
        varint::encode_signed(0, &mut buffer).unwrap();
        assert_eq!(buffer, vec![0x00]); // Special case: 0

        buffer.clear();
        varint::encode_signed(-1, &mut buffer).unwrap();
        assert_eq!(buffer, vec![0x01]); // Negative numbers zigzag encoded correctly

        buffer.clear();
        varint::encode_signed(1, &mut buffer).unwrap();
        assert_eq!(buffer, vec![0x02]);

        buffer.clear();
        varint::encode_signed(-2, &mut buffer).unwrap();
        assert_eq!(buffer, vec![0x03]); // Negative value -2

        buffer.clear();
        varint::encode_signed(i64::MAX, &mut buffer).unwrap();
        assert_eq!(buffer.len(), 10); // Maximum i64 uses 10 bytes in varint encoding

        buffer.clear();
        varint::encode_signed(i64::MIN, &mut buffer).unwrap();
        assert_eq!(buffer.len(), 10); // Minimum i64 also uses 10 bytes
    }

    #[test]
    fn test_header_write_and_read() {
        let assets = vec![
            "BTCUSDT".to_string(),
            "ETHUSDT".to_string(),
            "SOLUSDT".to_string(),
        ];

        let mut encoder = BinaryFormat::new().with_assets(assets.clone()).unwrap();
        let mut buffer = Vec::new();

        // Reference values
        let reference_timestamp = 1700000000000;
        let reference_prices = vec![45000.0, 2500.5, 120.75];
        let reference_quantities = vec![1.0, 10.0, 100.0];

        // Write header
        encoder
            .write_header(
                &mut buffer,
                reference_timestamp,
                &reference_prices,
                &reference_quantities,
            )
            .unwrap();

        // Read header
        let mut decoder = BinaryFormat::new();
        decoder.read_header(&mut Cursor::new(&buffer)).unwrap();

        // Assert that the header values are correct
        assert_eq!(decoder.assets, assets);
        assert_eq!(decoder.states.len(), assets.len());
        assert_eq!(decoder.states[0].last_timestamp, reference_timestamp);
        assert_eq!(decoder.states[0].last_price, reference_prices[0]);
        assert_eq!(decoder.states[0].last_quantity, reference_quantities[0]);
    }

    #[test]
    fn test_single_trade_encoding_and_decoding() {
        let assets = vec![
            "BTCUSDT".to_string(),
            "ETHUSDT".to_string(),
            "SOLUSDT".to_string(),
        ];

        let mut encoder = BinaryFormat::new().with_assets(assets.clone()).unwrap();
        let mut buffer = Vec::new();

        // Write header
        let reference_timestamp = 1700000000000;
        let reference_prices = vec![45000.0, 2500.5, 120.75];
        let reference_quantities = vec![1.0, 10.0, 100.0];
        encoder
            .write_header(
                &mut buffer,
                reference_timestamp,
                &reference_prices,
                &reference_quantities,
            )
            .unwrap();

        // Encode a single trade
        let trade = Trade {
            symbol: "BTCUSDT".to_string(),
            timestamp: 1700000001000, // Delta = +1000 ms
            price: 45001.0,           // Delta = +1.0
            quantity: 1.5,            // Delta = +0.5
            is_buyer_maker: true,
        };

        let encoded_trade = encoder.encode(&trade).unwrap();
        buffer.extend_from_slice(&encoded_trade);

        // Decode the trade
        let mut decoder = BinaryFormat::new();
        let mut cursor = Cursor::new(&buffer);
        decoder.read_header(&mut cursor).unwrap();
        let decoded_trade = decoder.read_message(&mut cursor).unwrap();

        // Compare decoded trade with the original
        assert_eq!(decoded_trade.symbol, trade.symbol);
        assert_eq!(decoded_trade.timestamp, trade.timestamp);
        assert!((decoded_trade.price - trade.price).abs() < 0.01);
        assert!((decoded_trade.quantity - trade.quantity).abs() < 0.00001);
        assert_eq!(decoded_trade.is_buyer_maker, trade.is_buyer_maker);
    }

    #[test]
    fn test_batch_trade_encoding_and_decoding() {
        let assets = vec![
            "BTCUSDT".to_string(),
            "ETHUSDT".to_string(),
            "SOLUSDT".to_string(),
        ];

        let mut encoder = BinaryFormat::new().with_assets(assets.clone()).unwrap();
        let mut buffer = Vec::new();

        // Write header
        let reference_timestamp = 1700000000000;
        let reference_prices = vec![45000.0, 2500.5, 120.75];
        let reference_quantities = vec![1.0, 10.0, 100.0];
        encoder
            .write_header(
                &mut buffer,
                reference_timestamp,
                &reference_prices,
                &reference_quantities,
            )
            .unwrap();

        // Prepare a batch of trades
        let trades = vec![
            Trade {
                symbol: "BTCUSDT".to_string(),
                timestamp: 1700000001000, // Delta = +1000 ms
                price: 45001.0,           // Delta = +1.0
                quantity: 1.5,            // Delta = +0.5
                is_buyer_maker: true,
            },
            Trade {
                symbol: "ETHUSDT".to_string(),
                timestamp: 1700000002000, // Delta = +2000 ms
                price: 2501.5,            // Delta = +1.0
                quantity: 10.5,           // Delta = +0.5
                is_buyer_maker: false,
            },
            Trade {
                symbol: "SOLUSDT".to_string(),
                timestamp: 1700000003000, // Delta = +3000 ms
                price: 121.0,             // Delta = +0.25
                quantity: 100.25,         // Delta = +0.25
                is_buyer_maker: true,
            },
        ];

        for trade in &trades {
            let encoded_trade = encoder.encode(trade).unwrap();
            buffer.extend_from_slice(&encoded_trade);
        }

        // Decode trades
        let mut decoder = BinaryFormat::new();
        let mut cursor = Cursor::new(&buffer);
        decoder.read_header(&mut cursor).unwrap();

        let mut decoded_trades = Vec::new();
        while cursor.position() < buffer.len() as u64 {
            let decoded_trade = decoder.read_message(&mut cursor).unwrap();
            decoded_trades.push(decoded_trade);
        }

        // Verify decoded trades
        assert_eq!(trades.len(), decoded_trades.len());
        for (original, decoded) in trades.iter().zip(decoded_trades.iter()) {
            assert_eq!(original.symbol, decoded.symbol);
            assert_eq!(original.timestamp, decoded.timestamp);
            assert!((original.price - decoded.price).abs() < 0.01);
            assert!((original.quantity - decoded.quantity).abs() < 0.00001);
            assert_eq!(original.is_buyer_maker, decoded.is_buyer_maker);
        }
    }
}
