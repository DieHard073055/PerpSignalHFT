use perp_signal_hft::format::{BinaryFormat, Trade};
use std::io::Cursor;
use std::io::{self, Read};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn read_buffered(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = loop {
        match TcpStream::connect("127.0.0.1:9000") {
            Ok(s) => {
                println!("Connected to server!");
                break s;
            }
            Err(_) => {
                thread::sleep(Duration::from_micros(10));
            }
        }
    };
    stream.set_nodelay(true)?;

    let start = read_buffered(&mut stream)?;
    assert_eq!(&start, b"START");
    println!("Client: received START");

    let header_buf = read_buffered(&mut stream)?;
    let mut decoder = BinaryFormat::new();
    decoder.read_header(&mut Cursor::new(&header_buf))?;
    println!("Client: read HEADER");

    loop {
        let data = read_buffered(&mut stream)?;
        let mut cursor = Cursor::new(&data);
        let trade: Trade = decoder.read_message(&mut cursor)?;

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        let latency = now.saturating_sub(trade.timestamp);
        println!("Client: {:?}, latency {} ms", trade, latency);
    }
}
