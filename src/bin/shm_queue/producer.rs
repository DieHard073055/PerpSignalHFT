use perp_signal_hft::ipc::shm_queue::ShmQueue;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{thread, time::Duration};

fn main() -> std::io::Result<()> {
    // Shared memory queue parameters
    let capacity = 1024 * 1024; // 1 MiB
    let queue_name = "trade_queue";

    // Initialize producer side
    let queue = ShmQueue::create(queue_name, capacity)?;

    // Send handshake
    queue.push(b"START")?;
    println!("Producer: sent START handshake");

    for i in 0..100 {
        // current time in microseconds
        let micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        // 8-byte little-endian timestamp
        queue.push(&micros.to_le_bytes())?;
        println!("Produced {}: {} µs", i, micros);

        thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}
