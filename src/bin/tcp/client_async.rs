use perp_signal_hft::format::BinaryFormat;
use std::io::Cursor;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut stream = TcpStream::connect("127.0.0.1:9000").await?;
    stream.set_nodelay(true)?;

    let start = read_buffered_async(&mut stream).await?;
    assert_eq!(&start, b"START");
    let header = read_buffered_async(&mut stream).await?;
    let mut decoder = BinaryFormat::new();

    decoder.read_header(&mut Cursor::new(&header))?;

    loop {
        let data = read_buffered_async(&mut stream).await?;
        let mut cur = Cursor::new(&data);
        let trade = decoder.read_message(&mut cur)?;
        println!("Client: {:?}, …", trade);
    }
}

async fn read_buffered_async(stream: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}
