# perp_signal_hft

Low-latency perp-trade forwarding service. Subscribes to Binance USDT-perpetual trade streams and fans them out to downstream consumers via TCP or a shared-memory ring buffer. Includes a compact binary encoding (`BinaryFormat`) for efficient transport.

[![asciicast: CI2KxepaSDZVqO7MlxDMbc90K](https://asciinema.org/a/CI2KxepaSDZVqO7MlxDMbc90K.svg)](https://asciinema.org/a/CI2KxepaSDZVqO7MlxDMbc90K)

## Table of Contents

- Features
- Assumptions
- Optimizations
- Getting Started
- CLI Usage
  - TCP Mode
  - SHM Mode
- Example Binaries
- Library Overview
- Modules
- Testing
- Contributing
- License

## Features

- **Real-time Binance WS**: Connects to Binance Futures perp trade streams with automatic reconnect/backoff.  
- **BinaryFormat**: Header + varint-encoded delta messages for minimal bandwidth.  
- **TCP Fan-out**: Broadcast or direct write to multiple TCP clients.  
- **Shared Memory IPC**: SPSC ring buffer under `/dev/shm` for sub-microsecond hand-off.  
- **REST Fallback**: Compute reference prices/quantities via Binance REST API for header initialization.  
- **Extensible CLI**: Subscribe up to 10 symbols; pick `tcp` or `shm` transport.  

## Assumptions
- The HFT strategy only needs the following fields:
  - timestamp
  - asset
  - price
  - quantity
  - is_buyer_maker
- We ignore the trade type flag `MARKET`, `ADL`, `INSURANCE_FUND`.
- The only assets we will subscribe to are USDT perps from `wss://fstream.binance.com/stream`
- We are only subscribing to the recent trades on the USDT perps.
- We are not going to subscribe to more than 10 perp pairs.
- Network connection is expected to be robust between binance -> this service -> downstream hft strategy.
  - binance websocket does have some retry logic.

## Optimizations
1. Delta-Varint Encoding
  - Zig-zag + varint for signed deltas in timestamps and prices, and unsigned for quantities.
  - Shrinks each trade message down to the minimal number of bytes
2. Header Pre-Calculation
  - Fetch reference prices/quantities via REST, build a full header blob.
  - Downstream clients only pay that cost once at startup.
3. Typed, Zero-Copy JSON Parsing
  - Custom Serde deserializer (`de_string_to_f64`) parses price/qty directly into f64.
  - Avoids intermediate string allocations and repeated parsing.
4. Exponential Backoff + Auto-Reconnect
  - Wrap `connect async` in a reusable `retry_with_backoff`
  - Automatic retries on network hiccups without busy-spinning
5. Shared-Memory Ring Buffer
  - Incase the downstream component is running in the same host.
  - `ShmQueue` in `/dev/shm` with atomic head/tail, no syscall on push/pop.
  - Sub-microsecond hand-off between processes.
6. Disable Nagle (`TCP_NODELAY`)
  - Ensures low-latency on every small write by turning off packet coalescing.

Binary message layout:
```
┌───────────────────────────────────────────────────────────────────────────────┐
│                               HEADER (variable size)                         │
├─────────┬────────┬─────────────┬───────────┬─────────────────────────────────┤
│ version │ #assets│ asset[0]    │ asset[1]  │  …                               │
│ (1 B)   │  (1 B) │ len + name  │ len+name  │                                 │
├───────────────────────────────────────────────────────────────────────────────┤
│ reference_timestamp (8 B little-endian)                                      │
├───────────────────────────────────────────────────────────────────────────────┤
│ reference_prices[0] … prices[N-1]  (each 8 B little-endian f64)              │
├───────────────────────────────────────────────────────────────────────────────┤
│ reference_quantities[0] … quantities[N-1]  (each 8 B little-endian f64)      │
└───────────────────────────────────────────────────────────────────────────────┘


┌───────────────────────────────────────────────────────────────────────────────┐
│                               TRADE MESSAGE                                 │
├───────────────┬──────────────────┬─────────────────────┬─────────────────────┤
│ symbol_id +   │ timestamp_delta  │ price_delta        │ quantity_fixed     │
│ buyer_maker   │ (signed varint)  │ (signed varint)    │ (unsigned varint)   │
│ (1 B)         │                  │                     │                     │
└───────────────┴──────────────────┴─────────────────────┴─────────────────────┘

Details:

HEADER:
┌────────┬───────┐
│ 0x01   │0x03   │  ← version=1, 3 assets
└────────┴───────┘

Asset entries (for “BTCUSDT”, “ETHUSDT”, “SOLUSDT”):
┌───────┬─────────────┐   ┌───────┬─────────────┐   ┌───────┬─────────────┐
│0x06   │"BTCUSDT"    │   │0x06   │"ETHUSDT"    │   │0x06   │"SOLUSDT"    │
└───────┴─────────────┘   └───────┴─────────────┘   └───────┴─────────────┘

Reference timestamp:
┌──────────────────────────────────────────────────┐
│ 0x00 0x00 0x01 0x85 0xF2 0xA0 0x00 0x00          │  ← e.g. 1700000000000
└──────────────────────────────────────────────────┘

Reference prices (each f64 LE):
┌──────────────────────────────────────────────────┐
│ 0x40 B0 F8 54 …  (45000.0), etc.                 │
└──────────────────────────────────────────────────┘

Reference quantities (each f64 LE):
┌──────────────────────────────────────────────────┐
│ 0x00 00 00 00 …  (0.0), etc.                     │
└──────────────────────────────────────────────────┘

TRADE MESSAGE:
┌─────────┐───────────────┬───────────────┬─────────────┐
│ 0x81    │ 0x8E 0x02     │ 0xAC 0x02     │ 0x96 0x01   │
│ (symbol │ timestamp     │ price_delta   │ quantity    │
│  id=1,  │ delta=270     │ delta=300     │ fixed=150   │
│ maker=1)│               │               │             │
└─────────┴───────────────┴───────────────┴─────────────┘
```
## Getting Started

1. Clone and build:

```shell
git clone https://github.com/your-org/perp_signal_hft.git
cd perp_signal_hft
cargo build --release
```

2. The main executable is `perp_signal_hft` under `target/release`.

## CLI Usage

```shell
USAGE: perp_signal_hft --assets BTCUSDT,ETHUSDT [--assets …] <SUBCOMMAND>

ARGS:
  --assets <assets>   Comma-delimited USDT-perp symbols (max 10)

SUBCOMMANDS:
  tcp    Fan out trades over TCP
  shm    Fan out trades via shared memory ring buffer
```

### Demo

Commands used in the demo. Please run the commands in the following order.

- TCP Demo
```shell
# First shell was running
./target/release/tcp-c

# Second shell was running
./target/release/perp_signal_hft --assets BTCUSDT,ETHUSDT,SOLUSDT tcp --port 9000
```
- Shm Demo

```shell
# First shell was running
./target/release/shm-q-cb

# Second shell was running
./target/release/perp_signal_hft --assets BTCUSDT,ETHUSDT,SOLUSDT shm --name trade_queue --capacity 1048576
```

### TCP Mode

Start a TCP server on port 9000:

```shell
target/release/perp_signal_hft \
  --assets BTCUSDT,ETHUSDT \
  tcp --port 9000
```

Clients can connect at `0.0.0.0:9000`, receive a `START` handshake, then a binary header, then framed trade messages.

### SHM Mode

Publish trades into a shared-memory queue named `trade_queue` of size 1 MiB:

```shell
target/release/perp_signal_hft \
  --assets BTCUSDT,ETHUSDT \
  shm --name trade_queue --capacity 1048576
```

Consumers can `pop()` length-prefixed messages from `/dev/shm/trade_queue`.

## Example Binaries

- **binary-format**  
  Demo of header + trade encode/decode loop.  
```shell
  cargo run --release --bin binary-format
```

- **binance-websockets**  
  Prints debug trade messages and per-minute counts.  
```shell
  cargo run --release --bin binance-websockets
```

- **shm-queue**  
  Simple producer/consumer of string messages via SHM.  
```shell
  cargo run --release --bin shm-queue
```

- **shm-q-pb / shm-q-cb / shm-q-p / shm-q-c**  
  Producer/consumer variants demonstrating timestamped trades.

- **tcp-s**  
  Standalone TCP server sending synthetic trades.  
```shell
  cargo run --release --bin tcp-s
```

- **tcp-c / tcp-c-a**  
  Sync and async TCP clients that connect, handshake, and print trades.

## Library Overview

The `perp_signal_hft` crate exposes:

- **format**:  
  - `BinaryFormat` – header + delta-varint encoding  
  - `varint` module – unsigned/signed encode & decode  
  - Extensive unit tests  

- **binance**:  
  - `TradeMessage` – parses WS JSON into `Trade`  
  - `retry_with_backoff` – reconnect logic  
  - `BinanceWebsocket` – WS subscription with ping/pong & backoff  
  - `BinanceClient` – REST endpoint for reference price/qty averages  

- **ipc**:  
  - `shm_queue::ShmQueue` – SPSC ring buffer via `memmap2` & atomics  
  - `tcp` – broadcast server & direct fan-out server  

- **cli**:  
  - Clap-based `Cli` & `Comm` for configuration  

## Modules

```shell
src/
├── binance.rs       # WS + REST clients
├── cli.rs           # CLI parsing
├── format.rs        # BinaryFormat & varint encoding
├── ipc/
│   ├── mod.rs
│   ├── shm_queue.rs # shared-memory queue
│   └── tcp.rs       # TCP fan-out
└── main.rs          # CLI wiring & pipeline orchestration
```

## Testing

Run the full test suite:

```shell
cargo test
```

Key tests live in `format.rs` covering varint edge cases, header round-trip, and message encoding/decoding.

## Contributing

1. Fork the repo.  
2. Create a feature branch.  
3. Submit a PR with clear descriptions and tests.  

Please follow the existing style and add unit tests for new functionality.

## License

This project is released under the [MIT License](LICENSE).  

