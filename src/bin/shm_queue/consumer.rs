use perp_signal_hft::ipc::shm_queue::ShmQueue;
use std::hint;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() -> std::io::Result<()> {
    // Shared memory queue must match producer
    let capacity = 1024 * 1024;
    let queue_name = "trade_queue";
    let queue = ShmQueue::create(queue_name, capacity)?;

    // Spin-wait for START handshake without sleeping
    loop {
        if let Some(data) = queue.pop()? {
            if data == b"START" {
                println!("Consumer: received START handshake");
                break;
            }
        }
        hint::spin_loop();
    }

    let mut out = vec![];

    // Process timestamped messages with busy spin
    for count in 0..100 {
        // Wait for next message
        let data = loop {
            if let Some(d) = queue.pop()? {
                break d;
            }
            hint::spin_loop();
        };

        // Compute latency
        let sent = u128::from_le_bytes(data.try_into().unwrap());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        out.push(format!("Consumed {}: {} ns latency", count, now - sent));
    }
    for o in out {
        println!("{}", o);
    }

    Ok(())
}
