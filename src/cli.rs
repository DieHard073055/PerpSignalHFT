use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "perp_signal_hft",
    version,
    about = "Low-latency perp trade forward service"
)]
pub struct Cli {
    /// List of usdt perp symbols to subscribe to (eg: BTCUSDT). Upto 10.
    #[clap(short, long, value_delimiter = ',', required = true)]
    pub assets: Vec<String>,

    /// Communication protocol
    #[command(subcommand)]
    pub comm: Comm,
}

#[derive(Debug, Subcommand)]
pub enum Comm {
    /// Use tcp socket
    Tcp {
        /// Port to bind on (0.0.0.0:<port>)
        #[clap(short, long)]
        port: u16,
    },
    /// Use shared memory ring buffer via /dev/shm
    Shm {
        /// Name of the shared memory queue (file in /dev/shm)
        #[clap(short, long)]
        name: String,

        /// Capacity of ring buffer in bytes
        #[clap(short, long, default_value = "1048576")]
        capacity: u32,
    },
}
