# kaspad-stratum

## Installation
- Install [Rust](http://rustup.rs)
- Checkout repository and `cd` to the folder
- Run `cargo build --release` 
- The binary will be in `target/release/`

## Usage
To start, simply run
```commandline
kaspad-stratum -m <KASPA_WALLET_ADDRESS> -r <KASPAD_RPC_URL>
```
This will start a stratum server at `127.0.0.1:6969`.

Additional options:
- `-s <IP:PORT>`:  change the stratum server address
- `-e <EXTRA_DATA>`: change the extra data
- `-d`: show debug output
