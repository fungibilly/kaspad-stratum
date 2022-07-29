# kaspad-stratum

## Installation
- Install [Rust](http://rustup.rs)
- Checkout repository and `cd` to the folder
- Run `cargo build --release` 
- The binary will be in `targ et/release/`

## Usage
To start, simply run
```commandline
kaspad-stratum -m <KASPA_WALLET_ADDRESS> -r <KASPAD_RPC_URL>
```
This will start a stratum server at `127.0.0.1:6969`.
Use the `-s` option to change the listen address.
Use the `-d` flag to display debug output.
