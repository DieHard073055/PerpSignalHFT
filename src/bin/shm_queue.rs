use perp_signal_hft::ipc::shm_queue::ShmQueue;
use std::{thread, time::Duration};

fn main() -> std::io::Result<()> {
    // Total capacity for messages (in bytes)
    let capacity = 1024 * 1024; // 1 MiB

    // Both producer and consumer open the same shared queue
    let producer_queue = ShmQueue::create("trade_queue", capacity)?;
    let consumer_queue = ShmQueue::create("trade_queue", capacity)?;

    // Spawn a producer thread
    let producer = thread::spawn(move || {
        for i in 0..5 {
            let msg = format!("trade-{}", i).into_bytes();
            producer_queue.push(&msg).expect("push failed");
            println!("Produced: trade-{}", i);
            thread::sleep(Duration::from_millis(100));
        }
    });

    // Spawn a consumer thread
    let consumer = thread::spawn(move || {
        let mut received = 0;
        while received < 5 {
            if let Some(data) = consumer_queue.pop().expect("pop failed") {
                let text = String::from_utf8(data).expect("invalid utf8");
                println!("Consumed: {}", text);
                received += 1;
            } else {
                // no message yet, wait a bit
                thread::sleep(Duration::from_millis(50));
            }
        }
    });

    producer.join().unwrap();
    consumer.join().unwrap();

    Ok(())
}
